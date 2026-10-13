//! Model Library: browse/download GGUF models from Hugging Face, manage the local
//! llama.cpp runtime (download + run `llama-server`), and list Ollama models.

use crate::ai::client;
use crate::config::{human_size, Secrets, Settings, LOCAL_LLAMA_PORT};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const FEATURED: &[(&str, &str)] = &[
    ("Qwen/Qwen2.5-Coder-7B-Instruct-GGUF", "Coding · 7B · great for scripts"),
    ("unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF", "Agentic coding MoE · 3B active"),
    ("Qwen/Qwen3-8B-GGUF", "General · 8B · tool use"),
    ("unsloth/gemma-3-4b-it-GGUF", "Gemma · 4B · fast on laptops"),
    ("bartowski/Llama-3.2-3B-Instruct-GGUF", "Llama · 3B · tiny & quick"),
];

#[derive(Clone)]
pub struct HfModel {
    pub id: String,
    pub downloads: u64,
    pub likes: u64,
}

#[derive(Clone)]
pub struct HfFile {
    pub path: String,
    pub size: u64,
}

type Slot<T> = Arc<Mutex<Option<Result<T, String>>>>;

pub struct Download {
    pub label: String,
    pub total: AtomicU64,
    pub done: AtomicU64,
    pub cancel: AtomicBool,
    pub status: Mutex<DlStatus>,
}

#[derive(Clone, PartialEq)]
pub enum DlStatus {
    Running,
    Finished,
    Failed(String),
}

impl Download {
    pub fn progress(&self) -> f32 {
        let t = self.total.load(Ordering::Relaxed);
        if t == 0 { 0.0 } else { self.done.load(Ordering::Relaxed) as f32 / t as f32 }
    }
}

#[derive(Clone, PartialEq)]
pub enum RuntimeState {
    Stopped,
    Starting(String),
    Ready(String),
    Failed(String),
}

pub struct Library {
    query: String,
    results: Slot<Vec<HfModel>>,
    searching: bool,
    open_repo: Option<String>,
    files: Slot<Vec<HfFile>>,
    pub downloads: Vec<Arc<Download>>,
    installed: Vec<(String, u64)>,
    ollama: Slot<Vec<String>>,
    runtime_dl: Option<Arc<Download>>,
    llama: Option<Child>,
    pub runtime_state: Arc<Mutex<RuntimeState>>,
    tab: LibTab,
}

#[derive(PartialEq, Clone, Copy)]
enum LibTab {
    Cloud,
    Local,
    HuggingFace,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            query: String::new(),
            results: Arc::new(Mutex::new(None)),
            searching: false,
            open_repo: None,
            files: Arc::new(Mutex::new(None)),
            downloads: vec![],
            installed: vec![],
            ollama: Arc::new(Mutex::new(None)),
            runtime_dl: None,
            llama: None,
            runtime_state: Arc::new(Mutex::new(RuntimeState::Stopped)),
            tab: LibTab::Cloud,
        }
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        self.stop_llama();
    }
}

/// What the UI asked the app to do.
pub enum LibAction {
    UseModel(String),
    Log(String),
}

fn get_json(url: &str, token: &str) -> Result<Value, String> {
    let mut r = client::agent().get(url);
    if !token.is_empty() {
        r = r.set("Authorization", &format!("Bearer {token}"));
    }
    match r.call() {
        Ok(resp) => resp.into_json().map_err(|e| e.to_string()),
        Err(ureq::Error::Status(c, _)) => Err(format!("HTTP {c}")),
        Err(e) => Err(e.to_string()),
    }
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Resumable streaming download into `dest` (via `dest.part`).
pub fn start_download(label: String, url: String, dest: PathBuf, token: String, ctx: egui::Context) -> Arc<Download> {
    let dl = Arc::new(Download {
        label,
        total: AtomicU64::new(0),
        done: AtomicU64::new(0),
        cancel: AtomicBool::new(false),
        status: Mutex::new(DlStatus::Running),
    });
    let d = dl.clone();
    std::thread::spawn(move || {
        let res = (|| -> Result<(), String> {
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let part = dest.with_extension(format!(
                "{}.part",
                dest.extension().and_then(|e| e.to_str()).unwrap_or("bin")
            ));
            let existing = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            let agent = ureq::AgentBuilder::new()
                .timeout_connect(std::time::Duration::from_secs(20))
                .timeout_read(std::time::Duration::from_secs(60))
                .build();
            let mut req = agent.get(&url).set("User-Agent", "Forge");
            if !token.is_empty() {
                req = req.set("Authorization", &format!("Bearer {token}"));
            }
            if existing > 0 {
                req = req.set("Range", &format!("bytes={existing}-"));
            }
            let resp = req.call().map_err(|e| e.to_string())?;
            let resumed = resp.status() == 206;
            let len: u64 = resp.header("Content-Length").and_then(|l| l.parse().ok()).unwrap_or(0);
            let start = if resumed { existing } else { 0 };
            d.total.store(start + len, Ordering::Relaxed);
            d.done.store(start, Ordering::Relaxed);
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(resumed)
                .truncate(!resumed)
                .open(&part)
                .map_err(|e| e.to_string())?;
            let mut reader = resp.into_reader();
            let mut buf = vec![0u8; 1 << 20];
            let mut last_repaint = std::time::Instant::now();
            loop {
                if d.cancel.load(Ordering::Relaxed) {
                    return Err("cancelled".into());
                }
                let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                d.done.fetch_add(n as u64, Ordering::Relaxed);
                if last_repaint.elapsed().as_millis() > 150 {
                    ctx.request_repaint();
                    last_repaint = std::time::Instant::now();
                }
            }
            file.flush().map_err(|e| e.to_string())?;
            drop(file);
            std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
            Ok(())
        })();
        *d.status.lock().unwrap() = match res {
            Ok(()) => DlStatus::Finished,
            Err(e) => DlStatus::Failed(e),
        };
        ctx.request_repaint();
    });
    dl
}

impl Library {
    pub fn refresh_installed(&mut self, settings: &Settings) {
        self.installed.clear();
        if let Ok(rd) = std::fs::read_dir(&settings.models_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("gguf") {
                    let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                    self.installed.push((p.file_name().unwrap().to_string_lossy().to_string(), size));
                }
            }
        }
        self.installed.sort();
    }

    pub fn installed_models(&self) -> Vec<String> {
        self.installed.iter().map(|(n, _)| format!("local:{n}")).collect()
    }

    pub fn ollama_models(&self) -> Vec<String> {
        match &*self.ollama.lock().unwrap() {
            Some(Ok(v)) => v.iter().map(|m| format!("ollama:{m}")).collect(),
            _ => vec![],
        }
    }

    fn search(&mut self, q: String, token: String, ctx: &egui::Context) {
        self.searching = true;
        let slot = self.results.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let url = format!(
                "https://huggingface.co/api/models?search={}&filter=gguf&sort=downloads&direction=-1&limit=40",
                url_encode(&q)
            );
            let r = get_json(&url, &token).map(|v| {
                v.as_array()
                    .map(|a| {
                        a.iter()
                            .map(|m| HfModel {
                                id: m["id"].as_str().unwrap_or_default().to_string(),
                                downloads: m["downloads"].as_u64().unwrap_or(0),
                                likes: m["likes"].as_u64().unwrap_or(0),
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            });
            *slot.lock().unwrap() = Some(r);
            ctx.request_repaint();
        });
    }

    fn list_files(&mut self, repo: String, token: String, ctx: &egui::Context) {
        self.open_repo = Some(repo.clone());
        *self.files.lock().unwrap() = None;
        let slot = self.files.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let url = format!("https://huggingface.co/api/models/{repo}/tree/main?recursive=true");
            let r = get_json(&url, &token).map(|v| {
                let mut files: Vec<HfFile> = v
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter(|f| f["type"] == "file" && f["path"].as_str().map_or(false, |p| p.ends_with(".gguf")))
                            .map(|f| HfFile {
                                path: f["path"].as_str().unwrap().to_string(),
                                size: f["lfs"]["size"].as_u64().or(f["size"].as_u64()).unwrap_or(0),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                files.sort_by_key(|f| f.size);
                files
            });
            *slot.lock().unwrap() = Some(r);
            ctx.request_repaint();
        });
    }

    pub fn refresh_ollama(&self, settings: &Settings, ctx: &egui::Context) {
        let base = settings.ollama_url.trim_end_matches('/').trim_end_matches("/v1").to_string();
        let slot = self.ollama.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = get_json(&format!("{base}/api/tags"), "").map(|v| {
                v["models"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|m| m["name"].as_str().map(str::to_string)).collect())
                    .unwrap_or_default()
            });
            *slot.lock().unwrap() = Some(r);
            ctx.request_repaint();
        });
    }

    // ------------------------------------------------------------ llama.cpp runtime

    pub fn find_llama_server(settings: &Settings) -> Option<PathBuf> {
        if !settings.llama_server.trim().is_empty() {
            let p = PathBuf::from(settings.llama_server.trim());
            return p.is_file().then_some(p);
        }
        let exe = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
        find_file(&settings.runtime_dir, exe, 4)
    }

    /// Download the latest llama.cpp release for this platform and unpack it into the runtime dir.
    pub fn install_runtime(&mut self, settings: &Settings, ctx: &egui::Context) {
        let runtime_dir = settings.runtime_dir.clone();
        let ctx2 = ctx.clone();
        let dl = Arc::new(Download {
            label: "llama.cpp runtime".into(),
            total: AtomicU64::new(0),
            done: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
            status: Mutex::new(DlStatus::Running),
        });
        self.runtime_dl = Some(dl.clone());
        std::thread::spawn(move || {
            let res = (|| -> Result<(), String> {
                let rel = get_json("https://api.github.com/repos/ggml-org/llama.cpp/releases/latest", "")?;
                let assets = rel["assets"].as_array().ok_or("no assets")?;
                let prefs: &[&str] = if cfg!(windows) {
                    &["bin-win-vulkan-x64.zip", "bin-win-cpu-x64.zip", "bin-win-avx2-x64.zip"]
                } else if cfg!(target_os = "macos") {
                    if cfg!(target_arch = "aarch64") { &["bin-macos-arm64"] } else { &["bin-macos-x64"] }
                } else {
                    &["bin-ubuntu-vulkan-x64", "bin-ubuntu-x64"]
                };
                let asset = prefs
                    .iter()
                    .find_map(|p| assets.iter().find(|a| a["name"].as_str().map_or(false, |n| n.contains(p))))
                    .ok_or("no matching llama.cpp build for this platform")?;
                let name = asset["name"].as_str().unwrap().to_string();
                let url = asset["browser_download_url"].as_str().unwrap().to_string();
                std::fs::create_dir_all(&runtime_dir).map_err(|e| e.to_string())?;
                let archive = runtime_dir.join(&name);
                let inner = start_download(name.clone(), url, archive.clone(), String::new(), ctx2.clone());
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    dl.total.store(inner.total.load(Ordering::Relaxed), Ordering::Relaxed);
                    dl.done.store(inner.done.load(Ordering::Relaxed), Ordering::Relaxed);
                    if dl.cancel.load(Ordering::Relaxed) {
                        inner.cancel.store(true, Ordering::Relaxed);
                    }
                    match inner.status.lock().unwrap().clone() {
                        DlStatus::Running => continue,
                        DlStatus::Failed(e) => return Err(e),
                        DlStatus::Finished => break,
                    }
                }
                let out = runtime_dir.join("llama");
                let _ = std::fs::remove_dir_all(&out);
                extract(&archive, &out)?;
                let _ = std::fs::remove_file(&archive);
                Ok(())
            })();
            *dl.status.lock().unwrap() = match res {
                Ok(()) => DlStatus::Finished,
                Err(e) => DlStatus::Failed(e),
            };
            ctx2.request_repaint();
        });
    }

    pub fn start_llama(&mut self, settings: &Settings, model_file: &str, ctx: &egui::Context) -> Result<(), String> {
        self.stop_llama();
        let server = Self::find_llama_server(settings).ok_or("llama-server not found — install the runtime first (Library ▸ Local).")?;
        let model = settings.models_dir.join(model_file);
        if !model.is_file() {
            return Err(format!("model file missing: {}", model.display()));
        }
        let log = std::fs::File::create(settings.runtime_dir.join("llama-server.log")).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&server);
        cmd.arg("-m")
            .arg(&model)
            .args(["--host", "127.0.0.1", "--port", &LOCAL_LLAMA_PORT.to_string()])
            .args(["-c", &settings.llama_ctx.to_string(), "-ngl", &settings.llama_gpu_layers.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::from(log));
        if let Some(dir) = server.parent() {
            cmd.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let child = cmd.spawn().map_err(|e| format!("failed to start llama-server: {e}"))?;
        self.llama = Some(child);
        *self.runtime_state.lock().unwrap() = RuntimeState::Starting(model_file.to_string());
        let state = self.runtime_state.clone();
        let ctx = ctx.clone();
        let mf = model_file.to_string();
        std::thread::spawn(move || {
            let url = format!("http://127.0.0.1:{LOCAL_LLAMA_PORT}/health");
            for _ in 0..600 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if *state.lock().unwrap() == RuntimeState::Stopped {
                    return;
                }
                if let Ok(r) = client::agent().get(&url).call() {
                    if r.status() == 200 {
                        *state.lock().unwrap() = RuntimeState::Ready(mf);
                        ctx.request_repaint();
                        return;
                    }
                }
            }
            *state.lock().unwrap() = RuntimeState::Failed("llama-server did not become ready (see runtime/llama-server.log)".into());
            ctx.request_repaint();
        });
        Ok(())
    }

    pub fn stop_llama(&mut self) {
        if let Some(mut c) = self.llama.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        *self.runtime_state.lock().unwrap() = RuntimeState::Stopped;
    }

    pub fn llama_running_model(&self) -> Option<String> {
        match &*self.runtime_state.lock().unwrap() {
            RuntimeState::Ready(m) | RuntimeState::Starting(m) => Some(m.clone()),
            _ => None,
        }
    }

    // ------------------------------------------------------------ UI

    pub fn runtime_progress_ui(&self, ui: &mut egui::Ui) {
        if let Some(dl) = &self.runtime_dl {
            let st = dl.status.lock().unwrap().clone();
            match st {
                DlStatus::Running => {
                    ui.add(egui::ProgressBar::new(dl.progress()).show_percentage().text("Downloading llama.cpp…"));
                }
                DlStatus::Failed(e) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("Runtime install failed: {e}"));
                }
                DlStatus::Finished => {}
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings, secrets: &Secrets, current_model: &str) -> Vec<LibAction> {
        let mut actions = vec![];
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, LibTab::Cloud, "☁ Cloud");
            ui.selectable_value(&mut self.tab, LibTab::Local, "💻 Local");
            ui.selectable_value(&mut self.tab, LibTab::HuggingFace, "⬇ HF Hub");
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| match self.tab {
            LibTab::Cloud => self.ui_cloud(ui, settings, current_model, &mut actions),
            LibTab::Local => self.ui_local(ui, settings, current_model, &mut actions),
            LibTab::HuggingFace => self.ui_hub(ui, settings, secrets, &mut actions),
        });
        // Finished downloads → refresh installed list.
        let before = self.downloads.len();
        self.downloads.retain(|d| {
            let st = d.status.lock().unwrap().clone();
            match st {
                DlStatus::Finished => {
                    actions.push(LibAction::Log(format!("Downloaded {}", d.label)));
                    false
                }
                _ => true,
            }
        });
        if before != self.downloads.len() {
            self.refresh_installed(settings);
        }
        actions
    }

    fn ui_cloud(&mut self, ui: &mut egui::Ui, settings: &mut Settings, current: &str, actions: &mut Vec<LibAction>) {
        ui.label(egui::RichText::new("OpenRouter models").strong());
        ui.small("Free tier · no download needed");
        ui.add_space(4.0);
        for m in crate::ai::models::CLOUD_MODELS {
            let active = current == m.id;
            model_card(ui, m.name, m.blurb, m.id, active, || actions.push(LibAction::UseModel(m.id.to_string())));
        }
        ui.add_space(6.0);
        ui.collapsing("Custom OpenRouter model", |ui| {
            ui.small("Any model id from openrouter.ai/models, e.g. vendor/model:free");
            let id = ui.make_persistent_id("custom_model_input");
            let mut text = ui.data_mut(|d| d.get_temp::<String>(id).unwrap_or_default());
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut text);
                if ui.button("Add").clicked() && !text.trim().is_empty() {
                    settings.custom_models.push(text.trim().to_string());
                    settings.save();
                    text.clear();
                }
            });
            ui.data_mut(|d| d.insert_temp(id, text));
            let mut remove = None;
            for (i, m) in settings.custom_models.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.selectable_label(current == m, m).clicked() {
                        actions.push(LibAction::UseModel(m.clone()));
                    }
                    if ui.small_button("✖").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                settings.custom_models.remove(i);
                settings.save();
            }
        });
    }

    fn ui_local(&mut self, ui: &mut egui::Ui, settings: &mut Settings, current: &str, actions: &mut Vec<LibAction>) {
        // Runtime
        ui.label(egui::RichText::new("llama.cpp runtime").strong());
        let server = Self::find_llama_server(settings);
        match &server {
            Some(p) => {
                ui.small(format!("✔ {}", p.file_name().unwrap().to_string_lossy()));
            }
            None => {
                ui.small("Not installed. Forge downloads the official llama.cpp build (Vulkan GPU on Windows/Linux, Metal on macOS).");
            }
        }
        self.runtime_progress_ui(ui);
        ui.horizontal(|ui| {
            let busy = self.runtime_dl.as_ref().map_or(false, |d| *d.status.lock().unwrap() == DlStatus::Running);
            if ui.add_enabled(!busy, egui::Button::new(if server.is_some() { "⟳ Update runtime" } else { "⬇ Install runtime" })).clicked() {
                self.install_runtime(settings, ui.ctx());
            }
            if self.llama.is_some() && ui.button("■ Stop server").clicked() {
                self.stop_llama();
            }
        });
        let state = self.runtime_state.lock().unwrap().clone();
        match state {
            RuntimeState::Stopped => {}
            RuntimeState::Starting(m) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.small(format!("Loading {m}…"));
                });
            }
            RuntimeState::Ready(m) => {
                ui.colored_label(egui::Color32::from_rgb(120, 220, 140), format!("● Serving {m} on :{LOCAL_LLAMA_PORT}"));
            }
            RuntimeState::Failed(e) => {
                ui.colored_label(egui::Color32::LIGHT_RED, e);
            }
        }
        ui.collapsing("Runtime settings", |ui| {
            ui.horizontal(|ui| {
                ui.label("Context");
                ui.add(egui::DragValue::new(&mut settings.llama_ctx).range(1024..=262144).speed(256));
            });
            ui.horizontal(|ui| {
                ui.label("GPU layers");
                ui.add(egui::DragValue::new(&mut settings.llama_gpu_layers).range(0..=999));
            });
            ui.label("Custom llama-server path (optional)");
            ui.text_edit_singleline(&mut settings.llama_server);
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Installed models").strong());
            if ui.small_button("⟳").on_hover_text("Rescan models folder").clicked() {
                self.refresh_installed(settings);
            }
        });
        ui.small(settings.models_dir.display().to_string());
        if self.installed.is_empty() {
            ui.small("No GGUF models yet — get some from the HF Hub tab.");
        }
        let mut delete = None;
        for (name, size) in self.installed.clone() {
            let id = format!("local:{name}");
            let active = current == id;
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new(&name).strong().color(if active { crate::theme::ACCENT } else { ui.visuals().text_color() }));
                ui.horizontal(|ui| {
                    ui.small(human_size(size));
                    if ui.small_button(if active { "✔ In use" } else { "▶ Use" }).clicked() {
                        match self.start_llama(settings, &name, ui.ctx()) {
                            Ok(()) => actions.push(LibAction::UseModel(id.clone())),
                            Err(e) => actions.push(LibAction::Log(e)),
                        }
                    }
                    if ui.small_button("🗑").on_hover_text("Delete file").clicked() {
                        delete = Some(name.clone());
                    }
                });
            });
        }
        if let Some(n) = delete {
            if self.llama_running_model().as_deref() == Some(n.as_str()) {
                self.stop_llama();
            }
            let _ = std::fs::remove_file(settings.models_dir.join(&n));
            self.refresh_installed(settings);
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Ollama").strong());
            if ui.small_button("⟳").clicked() {
                self.refresh_ollama(settings, ui.ctx());
            }
        });
        ui.horizontal(|ui| {
            ui.small("URL");
            ui.text_edit_singleline(&mut settings.ollama_url);
        });
        match &*self.ollama.lock().unwrap() {
            None => {
                ui.small("Press ⟳ to list models from a running Ollama.");
            }
            Some(Err(e)) => {
                ui.small(format!("Ollama not reachable ({e})"));
            }
            Some(Ok(list)) => {
                for m in list {
                    let id = format!("ollama:{m}");
                    if ui.selectable_label(current == id, format!("Ollama · {m}")).clicked() {
                        actions.push(LibAction::UseModel(id));
                    }
                }
            }
        }
    }

    fn ui_hub(&mut self, ui: &mut egui::Ui, settings: &mut Settings, secrets: &Secrets, actions: &mut Vec<LibAction>) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Search GGUF models…").desired_width(ui.available_width() - 44.0));
            if (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) | ui.button("🔍").clicked() {
                self.search(self.query.clone(), secrets.hf_token.clone(), &ctx);
            }
        });

        // Active downloads
        let mut remove = None;
        for (i, d) in self.downloads.iter().enumerate() {
            let st = d.status.lock().unwrap().clone();
            ui.horizontal(|ui| {
                ui.small(&d.label);
                if ui.small_button("✖").clicked() {
                    d.cancel.store(true, Ordering::Relaxed);
                    remove = Some(i);
                }
            });
            match st {
                DlStatus::Running => {
                    let txt = format!(
                        "{} / {}",
                        human_size(d.done.load(Ordering::Relaxed)),
                        human_size(d.total.load(Ordering::Relaxed))
                    );
                    ui.add(egui::ProgressBar::new(d.progress()).text(txt));
                }
                DlStatus::Failed(e) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("Failed: {e} (✖ to dismiss, retry resumes)"));
                }
                DlStatus::Finished => {}
            }
        }
        if let Some(i) = remove {
            self.downloads.remove(i);
        }

        let results = self.results.lock().unwrap().clone();
        if results.is_some() {
            self.searching = false;
        }
        if self.searching {
            ui.spinner();
        }
        let list: Vec<(String, String)> = match results {
            Some(Ok(r)) => r
                .into_iter()
                .map(|m| (m.id, format!("⬇ {}  ♥ {}", compact(m.downloads), compact(m.likes))))
                .collect(),
            Some(Err(e)) => {
                ui.colored_label(egui::Color32::LIGHT_RED, format!("Search failed: {e}"));
                vec![]
            }
            None => {
                ui.label(egui::RichText::new("Featured").strong());
                FEATURED.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
            }
        };
        for (repo, meta) in list {
            let open = self.open_repo.as_deref() == Some(repo.as_str());
            let resp = egui::Frame::group(ui.style())
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(egui::RichText::new(&repo).strong()).wrap());
                    ui.small(meta);
                    if open {
                        self.ui_files(ui, &repo, settings, secrets, &ctx, actions);
                    }
                })
                .response;
            if !open && resp.interact(egui::Sense::click()).clicked() {
                self.list_files(repo.clone(), secrets.hf_token.clone(), &ctx);
            }
        }
    }

    fn ui_files(&mut self, ui: &mut egui::Ui, repo: &str, settings: &Settings, secrets: &Secrets, ctx: &egui::Context, actions: &mut Vec<LibAction>) {
        let files = self.files.lock().unwrap().clone();
        match files {
            None => {
                ui.spinner();
            }
            Some(Err(e)) => {
                ui.colored_label(egui::Color32::LIGHT_RED, e);
            }
            Some(Ok(files)) => {
                if files.is_empty() {
                    ui.small("No .gguf files in this repo.");
                }
                for f in files {
                    let fname = f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
                    let recommended = fname.to_uppercase().contains("Q4_K_M");
                    let installed = settings.models_dir.join(&fname).is_file();
                    ui.horizontal(|ui| {
                        let mut t = egui::RichText::new(&fname).small();
                        if recommended {
                            t = t.color(crate::theme::GOLD);
                        }
                        ui.label(t).on_hover_text(if recommended { "Recommended quantization" } else { "" });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if installed {
                                ui.small("✔");
                            } else if ui.small_button("⬇").on_hover_text(human_size(f.size)).clicked() {
                                let url = format!("https://huggingface.co/{repo}/resolve/main/{}", url_encode(&f.path));
                                let dl = start_download(fname.clone(), url, settings.models_dir.join(&fname), secrets.hf_token.clone(), ctx.clone());
                                self.downloads.push(dl);
                                actions.push(LibAction::Log(format!("Downloading {fname} ({})", human_size(f.size))));
                            }
                            ui.small(human_size(f.size));
                        });
                    });
                }
            }
        }
    }
}

fn model_card(ui: &mut egui::Ui, name: &str, blurb: &str, id: &str, active: bool, mut on_use: impl FnMut()) {
    let stroke = if active { egui::Stroke::new(1.5_f32, crate::theme::ACCENT) } else { ui.visuals().widgets.noninteractive.bg_stroke };
    let r = egui::Frame::group(ui.style())
        .stroke(stroke)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(name).strong());
                if active {
                    ui.label(egui::RichText::new("● active").small().color(crate::theme::ACCENT));
                }
            });
            ui.small(blurb);
            ui.label(egui::RichText::new(id).small().weak().monospace());
        })
        .response;
    if r.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        on_use();
    }
}

fn compact(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

fn find_file(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut dirs = vec![];
    for e in rd.flatten() {
        let p = e.path();
        if p.is_file() && p.file_name().map_or(false, |n| n == name) {
            return Some(p);
        }
        if p.is_dir() {
            dirs.push(p);
        }
    }
    if depth == 0 {
        return None;
    }
    dirs.into_iter().find_map(|d| find_file(&d, name, depth - 1))
}

fn extract(archive: &Path, out: &Path) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let name = archive.to_string_lossy().to_lowercase();
    let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    if name.ends_with(".zip") {
        let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
        z.extract(out).map_err(|e| e.to_string())?;
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        tar::Archive::new(flate2::read::GzDecoder::new(f)).unpack(out).map_err(|e| e.to_string())?;
    } else {
        return Err(format!("unsupported archive {name}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(p) = find_file(out, "llama-server", 4) {
            let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(())
}
