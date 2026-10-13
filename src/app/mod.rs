//! The Forge editor: dockable Unity/Unreal-style layout + Antigravity-style agent panel.

mod content;
mod panels;
mod viewport;

use crate::ai::agent::Job;
use crate::ai::memory::Memory;
use crate::ai::tools::{self, ToolCall};
use crate::chat::{ChatPanel, ChatRequest};
use crate::config::{Secrets, Settings};
use crate::i18n::{self, t};
use crate::library::{LibAction, Library};
use crate::render::{FrameStats, GlRenderer, OrbitCamera, Shading};
use crate::runtime::{self, Runtime};
use crate::scene::{CameraComp, EntityId, Light, MeshKind, Scene};
use crate::start::{StartAction, StartScreen};
use crate::templates::Template;
use crate::theme::{self, ACCENT};
use crate::updater::Updater;
use crate::wizard::Wizard;
use egui::{Color32, RichText};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use glam::Vec3;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Scene,
    Game,
    Hierarchy,
    Library,
    Inspector,
    Lighting,
    ProjectSettings,
    Content,
    Console,
    Script,
}

impl Tab {
    pub const ALL: [Tab; 10] = [
        Tab::Scene,
        Tab::Game,
        Tab::Hierarchy,
        Tab::Library,
        Tab::Inspector,
        Tab::Lighting,
        Tab::ProjectSettings,
        Tab::Content,
        Tab::Console,
        Tab::Script,
    ];
    pub fn title(self) -> String {
        match self {
            Tab::Scene => format!("# {}", t("Scene", "Sahne")),
            Tab::Game => format!("🎮 {}", t("Game", "Oyun")),
            Tab::Hierarchy => format!("☰ {}", t("Hierarchy", "Hiyerarşi")),
            Tab::Library => format!("📚 {}", t("Library", "Kütüphane")),
            Tab::Inspector => format!("🔎 {}", t("Inspector", "Denetçi")),
            Tab::Lighting => format!("☀ {}", t("Lighting", "Işıklandırma")),
            Tab::ProjectSettings => format!("⚙ {}", t("Project Settings", "Proje Ayarları")),
            Tab::Content => format!("📁 {}", t("Content Browser", "İçerik Tarayıcı")),
            Tab::Console => format!("🖥 {}", t("Console", "Konsol")),
            Tab::Script => format!("📜 {}", t("Script", "Betik")),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Ai,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Move,
    Rotate,
    Scale,
}

pub struct GizmoDrag {
    pub axis: usize,
    pub tool: Tool,
    pub start: crate::scene::Transform,
    pub accumulated: f32,
    pub last_angle: f32,
}

#[derive(Default)]
pub struct Dialogs {
    pub open: Option<String>,
    pub save_as: Option<String>,
    pub build: Option<String>,
    pub settings: bool,
    pub scripting_help: bool,
    pub about: bool,
    pub script_goal: String,
    pub prefab_name: Option<(EntityId, String)>,
}

#[derive(PartialEq)]
enum Screen {
    Wizard,
    Start,
    Editor,
}

pub struct Editor {
    pub settings: Settings,
    pub secrets: Secrets,
    pub scene: Scene,
    pub scene_path: Option<PathBuf>,
    pub dirty: bool,
    pub selected: Option<EntityId>,
    pub undo: Vec<String>,
    pub redo: Vec<String>,
    pub editing: bool,
    pub cam: OrbitCamera,
    pub renderer: Option<Arc<Mutex<GlRenderer>>>,
    pub runtime: Option<Runtime>,
    pub console: Vec<(LogLevel, String)>,
    pub console_filter: [bool; 4],
    pub console_search: String,
    pub chat: ChatPanel,
    pub library: Library,
    pub dialogs: Dialogs,
    pub gizmo: Option<GizmoDrag>,
    pub tool: Tool,
    pub shading: Shading,
    pub hierarchy_filter: String,
    pub player_mode: bool,
    pub frame_ms: f32,
    pub fps: f32,
    pub stats: FrameStats,
    pub focus_tab: Option<Tab>,
    pub reset_layout: bool,
    pub open_tab: Option<Tab>,
    // Content browser
    pub cb_dir: PathBuf,
    pub cb_search: String,
    pub cb_selected: Option<PathBuf>,
    pub cb_new: Option<(u8, String)>,
    pub cb_import: Option<String>,
    pub thumbs: HashMap<PathBuf, egui::TextureHandle>,
    pub always_allow: Vec<String>,
    pub turn_undo: Option<String>,
    pub memory_status: Arc<Mutex<String>>,
}

pub struct ForgeApp {
    ed: Editor,
    dock: DockState<Tab>,
    screen: Screen,
    wizard: Option<Wizard>,
    start: Option<StartScreen>,
    updater: Updater,
    last_frame: Instant,
    last_title: String,
    last_update_check: Instant,
}

pub fn default_layout() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Scene, Tab::Game]);
    let surf = dock.main_surface_mut();
    let [center, _right] = surf.split_right(
        NodeIndex::root(),
        0.73,
        vec![Tab::Inspector, Tab::Lighting, Tab::ProjectSettings],
    );
    let [center, _left] = surf.split_left(center, 0.28, vec![Tab::Hierarchy, Tab::Library]);
    let [_center, _bottom] =
        surf.split_below(center, 0.68, vec![Tab::Content, Tab::Console, Tab::Script]);
    dock
}

impl ForgeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, player_scene: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        crate::updater::cleanup_old();
        let mut console = vec![];
        let renderer = match cc.gl.as_ref().map(|gl| GlRenderer::new(gl)) {
            Some(Ok(r)) => Some(Arc::new(Mutex::new(r))),
            Some(Err(e)) => {
                console.push((LogLevel::Error, format!("Renderer init failed: {e}")));
                None
            }
            None => None,
        };
        let settings = Settings::load();
        i18n::set(settings.lang);
        let secrets = Secrets::load();
        let mut library = Library::default();
        library.refresh_installed(&settings);
        let mut chat = ChatPanel::default();
        chat.web = settings.agent_web;
        chat.mode = settings.agent_mode;

        let player = player_scene.is_some();
        let screen = if player {
            Screen::Editor
        } else if !settings.setup_done {
            Screen::Wizard
        } else {
            Screen::Start
        };
        let ed = Editor {
            settings,
            secrets,
            scene: Scene::starter(),
            scene_path: None,
            dirty: false,
            selected: None,
            undo: vec![],
            redo: vec![],
            editing: false,
            cam: OrbitCamera::default(),
            renderer,
            runtime: None,
            console,
            console_filter: [true; 4],
            console_search: String::new(),
            chat,
            library,
            dialogs: Dialogs::default(),
            gizmo: None,
            tool: Tool::Move,
            shading: Shading::Lit,
            hierarchy_filter: String::new(),
            player_mode: player,
            frame_ms: 0.0,
            fps: 0.0,
            stats: FrameStats::default(),
            focus_tab: None,
            reset_layout: false,
            open_tab: None,
            cb_dir: PathBuf::new(),
            cb_search: String::new(),
            cb_selected: None,
            cb_new: None,
            cb_import: None,
            thumbs: HashMap::new(),
            always_allow: vec![],
            turn_undo: None,
            memory_status: Arc::new(Mutex::new(String::new())),
        };
        let mut app = Self {
            wizard: (screen == Screen::Wizard).then(|| Wizard::new(&ed.settings, &ed.secrets)),
            start: (screen == Screen::Start).then(|| StartScreen::new(&ed.settings)),
            ed,
            dock: default_layout(),
            screen,
            updater: Updater::default(),
            last_frame: Instant::now(),
            last_title: String::new(),
            last_update_check: Instant::now(),
        };
        if !player {
            app.updater.check(&cc.egui_ctx);
        }
        if let Some(p) = player_scene {
            match Scene::load(&p) {
                Ok(s) => app.ed.scene = s,
                Err(e) => app
                    .ed
                    .log(LogLevel::Error, format!("Could not load game: {e}")),
            }
            app.ed.play();
        }
        if app.ed.secrets.openrouter_key.is_empty() && !player {
            app.ed.log(
                LogLevel::Warn,
                t(
                    "No OpenRouter key yet — add it in AI › Settings.",
                    "OpenRouter anahtarı yok — AI › Ayarlar'dan ekleyin.",
                )
                .into(),
            );
        }
        app.ed.log(
            LogLevel::Info,
            format!(
                "Forge {} ready. Data: {}",
                env!("CARGO_PKG_VERSION"),
                crate::config::data_dir().display()
            ),
        );
        app
    }
}

impl Editor {
    pub fn log(&mut self, level: LogLevel, msg: String) {
        self.console.push((level, msg));
        if self.console.len() > 3000 {
            self.console.drain(0..800);
        }
    }

    pub fn project_root(&self) -> PathBuf {
        match &self.scene_path {
            Some(p) => p
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| self.settings.projects_dir.clone()),
            None => self.settings.projects_dir.join(sanitize(&self.scene.name)),
        }
    }

    // ------------------------------------------------------------ scene ops

    pub fn push_undo(&mut self) {
        self.undo.push(serde_json::to_string(&self.scene).unwrap());
        if self.undo.len() > 150 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    pub fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(serde_json::to_string(&self.scene).unwrap());
            self.scene = serde_json::from_str(&s).unwrap();
            self.dirty = true;
        }
    }

    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(serde_json::to_string(&self.scene).unwrap());
            self.scene = serde_json::from_str(&s).unwrap();
            self.dirty = true;
        }
    }

    pub fn create(&mut self, name: &str, mesh: MeshKind) -> EntityId {
        self.push_undo();
        let id = self.scene.spawn(name, mesh);
        let e = self.scene.get_mut(id).unwrap();
        let tg = self.cam.target;
        e.transform.position = Vec3::new(
            tg.x.round(),
            if mesh == MeshKind::Plane { 0.0 } else { 0.5 },
            tg.z.round(),
        );
        if mesh != MeshKind::None {
            e.color = [0.85, 0.85, 0.85];
        }
        self.selected = Some(id);
        id
    }

    pub fn delete_selected(&mut self) {
        if let Some(id) = self.selected.take() {
            self.push_undo();
            self.scene.remove(id);
        }
    }

    pub fn duplicate_selected(&mut self) {
        if let Some(id) = self.selected {
            self.push_undo();
            let sub = self.scene.subtree(id);
            let pos = self.scene.get(id).map(|e| e.transform.position + Vec3::X);
            self.selected = self.scene.instantiate(sub, pos);
            if let (Some(n), Some(p)) = (self.selected, self.scene.get(id).and_then(|e| e.parent)) {
                self.scene.get_mut(n).unwrap().parent = Some(p);
            }
        }
    }

    pub fn new_scene(&mut self, tp: Template) {
        self.push_undo();
        self.scene = tp.build();
        self.scene_path = None;
        self.selected = None;
    }

    pub fn open_scene(&mut self, p: PathBuf) {
        match Scene::load(&p) {
            Ok(s) => {
                self.scene = s;
                self.scene_path = Some(p.clone());
                self.selected = None;
                self.undo.clear();
                self.redo.clear();
                self.dirty = false;
                self.settings.last_scene = Some(p.clone());
                self.settings.save();
                crate::assets::ensure_project(&self.project_root());
                self.cb_dir = PathBuf::new();
                self.log(
                    LogLevel::Info,
                    format!("{} {}", t("Opened", "Açıldı:"), p.display()),
                );
            }
            Err(e) => self.log(
                LogLevel::Error,
                format!("{} {e}", t("Open failed:", "Açılamadı:")),
            ),
        }
    }

    pub fn save(&mut self) {
        match self.scene_path.clone() {
            Some(p) => self.save_to(p),
            None => {
                self.dialogs.save_as = Some(
                    self.settings
                        .projects_dir
                        .join(sanitize(&self.scene.name))
                        .join("main.scene.json")
                        .display()
                        .to_string(),
                )
            }
        }
    }

    pub fn save_to(&mut self, p: PathBuf) {
        match self.scene.save(&p) {
            Ok(()) => {
                self.scene_path = Some(p.clone());
                self.dirty = false;
                self.settings.last_scene = Some(p.clone());
                self.settings.save();
                crate::assets::ensure_project(&self.project_root());
                self.log(
                    LogLevel::Info,
                    format!("{} {}", t("Saved", "Kaydedildi:"), p.display()),
                );
            }
            Err(e) => self.log(
                LogLevel::Error,
                format!("{} {e}", t("Save failed:", "Kaydedilemedi:")),
            ),
        }
    }

    /// Export a standalone game: copy the Forge binary + scene (+ project assets) into a folder.
    pub fn build_game(&mut self, dir: PathBuf) {
        let root = self.project_root();
        let res = (|| -> std::io::Result<PathBuf> {
            std::fs::create_dir_all(&dir)?;
            let exe = std::env::current_exe()?;
            let name = sanitize(&self.scene.name).replace(' ', "");
            let name = if name.is_empty() {
                "Game".to_string()
            } else {
                name
            };
            let target = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
            std::fs::copy(&exe, &target)?;
            self.scene.save(&dir.join("game.scene.json"))?;
            let tex = root.join("Textures");
            if tex.is_dir() {
                std::fs::create_dir_all(dir.join("Textures"))?;
                for f in std::fs::read_dir(tex)?.flatten() {
                    let _ = std::fs::copy(f.path(), dir.join("Textures").join(f.file_name()));
                }
            }
            Ok(target)
        })();
        match res {
            Ok(tg) => self.log(
                LogLevel::Info,
                format!("{} {}", t("Game built:", "Oyun derlendi:"), tg.display()),
            ),
            Err(e) => self.log(LogLevel::Error, format!("Build failed: {e}")),
        }
    }

    pub fn save_prefab(&mut self, id: EntityId, name: &str) {
        let dir = self.project_root().join("Prefabs");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(format!("{}.prefab.json", sanitize(name)));
        let sub = self.scene.subtree(id);
        match std::fs::write(&p, serde_json::to_string_pretty(&sub).unwrap()) {
            Ok(()) => self.log(LogLevel::Info, format!("Prefab: {}", p.display())),
            Err(e) => self.log(LogLevel::Error, e.to_string()),
        }
    }

    pub fn instantiate_prefab(&mut self, p: &std::path::Path) {
        match std::fs::read_to_string(p)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<crate::scene::Entity>>(&s).ok())
        {
            Some(ents) => {
                self.push_undo();
                let at = Some(Vec3::new(
                    self.cam.target.x.round(),
                    ents.first().map_or(0.5, |e| e.transform.position.y),
                    self.cam.target.z.round(),
                ));
                self.selected = self.scene.instantiate(ents, at);
            }
            None => self.log(LogLevel::Error, format!("Invalid prefab {}", p.display())),
        }
    }

    // ------------------------------------------------------------ play mode

    pub fn play(&mut self) {
        if self.runtime.is_some() {
            return;
        }
        let (rt, errors) = Runtime::new(self.scene.clone());
        for e in errors {
            self.log(LogLevel::Error, e);
        }
        self.runtime = Some(rt);
        self.focus_tab = Some(Tab::Game);
        if !self.player_mode {
            self.log(LogLevel::Info, "▶ Play".into());
        }
    }

    pub fn stop(&mut self) {
        if self.runtime.take().is_some() {
            self.focus_tab = Some(Tab::Scene);
            self.log(
                LogLevel::Info,
                t("■ Stop — scene restored", "■ Durdur — sahne geri yüklendi").into(),
            );
        }
    }

    pub fn step_once(&mut self) {
        if let Some(rt) = &mut self.runtime {
            rt.paused = false;
            rt.step(1.0 / 60.0);
            rt.paused = true;
        }
    }

    fn tick_runtime(&mut self, ctx: &egui::Context, dt: f32) {
        let typing = ctx.wants_keyboard_input();
        let Some(rt) = &mut self.runtime else { return };
        let (down, pressed) = if typing {
            Default::default()
        } else {
            runtime::collect_input(ctx)
        };
        rt.set_input(down, pressed);
        let errors = rt.step(dt);
        let logs = rt.take_logs();
        for e in errors {
            self.log(LogLevel::Error, e);
        }
        for l in logs {
            self.log(LogLevel::Info, l);
        }
        ctx.request_repaint();
    }

    // ------------------------------------------------------------ AI

    pub fn select_model(&mut self, id: String, ctx: &egui::Context) {
        if let Some(file) = id.strip_prefix("local:") {
            if self.library.llama_running_model().as_deref() != Some(file) {
                if let Err(e) = self.library.start_llama(&self.settings, file, ctx) {
                    self.log(LogLevel::Error, e);
                    return;
                }
                self.log(LogLevel::Ai, format!("Starting local model {file}…"));
            }
        }
        self.settings.selected_model = id.clone();
        self.settings.save();
        self.log(
            LogLevel::Ai,
            format!("Model: {}", crate::ai::models::display_name(&id)),
        );
    }

    pub fn send_chat(&mut self, ctx: &egui::Context) {
        let prompt = self.chat.input.trim().to_string();
        if prompt.is_empty() || self.chat.busy() {
            return;
        }
        if self.runtime.is_some() {
            self.stop();
        }
        // Expand @mentions into explicit entity references.
        let mut prompt_full = prompt.clone();
        let mentioned: Vec<String> = prompt
            .split_whitespace()
            .filter_map(|w| w.strip_prefix('@'))
            .filter_map(|n| {
                self.scene
                    .find(&n.replace('_', " "))
                    .or_else(|| self.scene.find(n))
            })
            .filter_map(|id| {
                self.scene
                    .get(id)
                    .map(|e| serde_json::to_string(&e.clone()).unwrap_or_default())
            })
            .collect();
        if !mentioned.is_empty() {
            prompt_full.push_str(&format!(
                "\n\n[Mentioned entities]\n{}",
                mentioned.join("\n")
            ));
        }
        let selected = if self.chat.attach_selection {
            self.selected
                .and_then(|s| self.scene.get(s))
                .map(|e| e.name.clone())
        } else {
            None
        };
        self.settings.agent_web = self.chat.web;
        self.settings.agent_mode = self.chat.mode;
        self.turn_undo = Some(serde_json::to_string(&self.scene).unwrap());
        self.push_undo(); // one undo step for the whole agent run
        let job = Job {
            prompt: prompt_full,
            mode: self.chat.mode,
            history: self.chat.history_msgs(),
            scene_json: self.scene.summary(),
            selected,
            settings: self.settings.clone(),
            openrouter_key: self.secrets.openrouter_key.clone(),
            web: self.chat.web,
            memory: self.settings.memory.clone(),
            session_id: self.chat.conv.id.clone(),
            summary: self.chat.summary(),
        };
        self.chat.start(job, ctx);
    }

    pub fn exec_tool(&mut self, call: &ToolCall) -> Result<String, String> {
        let root = self.project_root();
        let r = tools::execute_in(&mut self.scene, call, Some(&root));
        match &r {
            Ok(o) => {
                if tools::is_mutation(&call.name) {
                    self.dirty = true;
                }
                if o.select.is_some() {
                    self.selected = o.select;
                }
                if call.name == "write_file" {
                    if let Some(r) = &self.renderer {
                        r.lock().unwrap().invalidate_textures();
                    }
                }
                self.log(LogLevel::Ai, format!("tool {} ✔", call.name));
            }
            Err(e) => self.log(LogLevel::Warn, format!("tool {} ✖ {e}", call.name)),
        }
        r.map(|o| o.result)
    }

    fn handle_chat_requests(&mut self, reqs: Vec<ChatRequest>, ctx: &egui::Context) {
        for r in reqs {
            match r {
                ChatRequest::Send => self.send_chat(ctx),
                ChatRequest::SelectModel(m) => self.select_model(m, ctx),
                ChatRequest::Settings => self.dialogs.settings = true,
                ChatRequest::Approve(allow, always) => {
                    let mut chat = std::mem::take(&mut self.chat);
                    if always {
                        if let Some(name) = chat.pending_name() {
                            self.always_allow.push(name);
                        }
                    }
                    chat.resolve(allow, |c| self.exec_tool(c));
                    self.chat = chat;
                }
                ChatRequest::UndoTurn => {
                    if let Some(s) = self.turn_undo.take() {
                        self.push_undo();
                        self.scene = serde_json::from_str(&s).unwrap();
                        self.log(
                            LogLevel::Ai,
                            t(
                                "Reverted the agent's changes.",
                                "Ajanın değişiklikleri geri alındı.",
                            )
                            .into(),
                        );
                    }
                }
                ChatRequest::Feedback(good) => {
                    if let Some((p, a)) = self.chat.set_feedback(good) {
                        // Feed explicit feedback into agent memory as a preference signal.
                        let mem = Memory::new(self.settings.memory.clone());
                        let fact = format!(
                            "User rated the answer to \"{}\" as {}. Answer: {}",
                            p.chars().take(200).collect::<String>(),
                            if good { "good" } else { "bad" },
                            a.chars().take(300).collect::<String>()
                        );
                        let sid = self.chat.conv.id.clone();
                        std::thread::spawn(move || {
                            let _ = mem.remember(&sid, &fact);
                        });
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ menus

    fn menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::menu::bar(ui, |ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            theme::draw_logo(ui.painter(), r);
            ui.menu_button(t("File", "Dosya"), |ui| {
                ui.menu_button(t("New Scene", "Yeni Sahne"), |ui| {
                    for tp in Template::ALL {
                        if ui.button(tp.label()).clicked() {
                            self.new_scene(tp);
                            ui.close_menu();
                        }
                    }
                });
                if ui.button(t("Open…", "Aç…")).clicked() {
                    self.dialogs.open = Some(self.settings.projects_dir.display().to_string());
                    ui.close_menu();
                }
                if ui
                    .add(egui::Button::new(t("Save", "Kaydet")).shortcut_text("Ctrl+S"))
                    .clicked()
                {
                    self.save();
                    ui.close_menu();
                }
                if ui.button(t("Save As…", "Farklı Kaydet…")).clicked() {
                    self.dialogs.save_as = Some(
                        self.scene_path
                            .clone()
                            .unwrap_or_else(|| {
                                self.settings
                                    .projects_dir
                                    .join("Untitled")
                                    .join("main.scene.json")
                            })
                            .display()
                            .to_string(),
                    );
                    ui.close_menu();
                }
                ui.separator();
                if ui.button(t("Build Game…", "Oyunu Derle…")).clicked() {
                    self.dialogs.build = Some(
                        self.settings
                            .projects_dir
                            .join("Builds")
                            .join(sanitize(&self.scene.name))
                            .display()
                            .to_string(),
                    );
                    ui.close_menu();
                }
                ui.separator();
                if ui.button(t("Quit", "Çıkış")).clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(t("Edit", "Düzen"), |ui| {
                if ui
                    .add_enabled(
                        !self.undo.is_empty(),
                        egui::Button::new(t("Undo", "Geri al")).shortcut_text("Ctrl+Z"),
                    )
                    .clicked()
                {
                    self.undo();
                    ui.close_menu();
                }
                if ui
                    .add_enabled(
                        !self.redo.is_empty(),
                        egui::Button::new(t("Redo", "Yinele")).shortcut_text("Ctrl+Y"),
                    )
                    .clicked()
                {
                    self.redo();
                    ui.close_menu();
                }
                ui.separator();
                if ui
                    .add_enabled(
                        self.selected.is_some(),
                        egui::Button::new(t("Duplicate", "Çoğalt")).shortcut_text("Ctrl+D"),
                    )
                    .clicked()
                {
                    self.duplicate_selected();
                    ui.close_menu();
                }
                if ui
                    .add_enabled(
                        self.selected.is_some(),
                        egui::Button::new(t("Delete", "Sil")).shortcut_text("Del"),
                    )
                    .clicked()
                {
                    self.delete_selected();
                    ui.close_menu();
                }
            });
            ui.menu_button(t("GameObject", "Nesne"), |ui| self.create_menu(ui));
            ui.menu_button(t("Window", "Pencere"), |ui| {
                for tab in Tab::ALL {
                    if ui.button(tab.title()).clicked() {
                        self.open_tab = Some(tab);
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.button(t("Reset layout", "Düzeni sıfırla")).clicked() {
                    self.reset_layout = true;
                    ui.close_menu();
                }
            });
            ui.menu_button("AI", |ui| {
                if ui.button(t("⚙ AI Settings…", "⚙ AI Ayarları…")).clicked() {
                    self.dialogs.settings = true;
                    ui.close_menu();
                }
                if ui
                    .button(t("📚 Model Library", "📚 Model Kütüphanesi"))
                    .clicked()
                {
                    self.open_tab = Some(Tab::Library);
                    ui.close_menu();
                }
                if ui
                    .add(
                        egui::Button::new(t("✨ Focus agent", "✨ Ajana odaklan"))
                            .shortcut_text("Ctrl+L"),
                    )
                    .clicked()
                {
                    self.chat.focus();
                    ui.close_menu();
                }
            });
            ui.menu_button(t("Help", "Yardım"), |ui| {
                if ui
                    .button(t("Scripting reference", "Betik başvurusu"))
                    .clicked()
                {
                    self.dialogs.scripting_help = true;
                    ui.close_menu();
                }
                if ui.button(t("About Forge", "Forge hakkında")).clicked() {
                    self.dialogs.about = true;
                    ui.close_menu();
                }
            });

            // Play controls, centered.
            let mid = ui.max_rect().center().x;
            let w = 120.0;
            let cur = ui.cursor().min.x;
            if mid - w / 2.0 > cur {
                ui.add_space(mid - w / 2.0 - cur);
            }
            let playing = self.runtime.is_some();
            let paused = self.runtime.as_ref().map_or(false, |r| r.paused);
            let play_btn = egui::Button::new(
                RichText::new(if playing { "■" } else { "▶" })
                    .size(14.0)
                    .color(if playing { ACCENT } else { theme::BONE }),
            )
            .fill(if playing { theme::BG3 } else { theme::BG2 })
            .min_size(egui::vec2(32.0, 22.0));
            if ui
                .add(play_btn)
                .on_hover_text(t("Play / Stop (Ctrl+P)", "Oynat / Durdur (Ctrl+P)"))
                .clicked()
            {
                if playing {
                    self.stop()
                } else {
                    self.play()
                }
            }
            if ui
                .add_enabled(
                    playing,
                    egui::Button::new(RichText::new("⏸").size(12.0).color(if paused {
                        ACCENT
                    } else {
                        theme::BONE
                    }))
                    .min_size(egui::vec2(32.0, 22.0)),
                )
                .on_hover_text(t("Pause", "Duraklat"))
                .clicked()
            {
                if let Some(r) = &mut self.runtime {
                    r.paused = !r.paused;
                }
            }
            if ui
                .add_enabled(
                    paused,
                    egui::Button::new(RichText::new("⏭").size(12.0))
                        .min_size(egui::vec2(32.0, 22.0)),
                )
                .on_hover_text(t("Step one frame", "Bir kare ilerle"))
                .clicked()
            {
                self.step_once();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{:.0} FPS", self.fps)).small().weak());
            });
        });
    }

    pub fn create_menu(&mut self, ui: &mut egui::Ui) {
        for (label, mesh) in [
            (t("Create Empty", "Boş Nesne"), MeshKind::None),
            ("Cube", MeshKind::Cube),
            ("Sphere", MeshKind::Sphere),
            ("Plane", MeshKind::Plane),
            ("Cylinder", MeshKind::Cylinder),
            ("Cone", MeshKind::Cone),
            ("Capsule", MeshKind::Capsule),
        ] {
            if ui.button(label).clicked() {
                let name = if mesh == MeshKind::None {
                    "GameObject"
                } else {
                    mesh.label()
                };
                self.create(name, mesh);
                ui.close_menu();
            }
        }
        ui.separator();
        if ui.button(t("Camera", "Kamera")).clicked() {
            let id = self.create("Camera", MeshKind::None);
            let e = self.scene.get_mut(id).unwrap();
            e.camera = Some(CameraComp::default());
            e.transform.position = Vec3::new(0.0, 4.0, 10.0);
            e.transform.rotation = Vec3::new(-15.0, 0.0, 0.0);
            ui.close_menu();
        }
        if ui.button(t("Directional Light", "Yönlü Işık")).clicked() {
            let id = self.create("Sun", MeshKind::None);
            let e = self.scene.get_mut(id).unwrap();
            e.light = Some(Light::default());
            e.transform.position.y = 8.0;
            e.transform.rotation = Vec3::new(-50.0, 30.0, 0.0);
            ui.close_menu();
        }
        if ui.button(t("Point Light", "Noktasal Işık")).clicked() {
            let id = self.create("Point Light", MeshKind::None);
            let e = self.scene.get_mut(id).unwrap();
            e.light = Some(Light::point());
            e.transform.position.y = 2.5;
            ui.close_menu();
        }
    }

    // ------------------------------------------------------------ dialogs

    fn dialogs_ui(&mut self, ctx: &egui::Context) {
        if let Some(mut path) = self.dialogs.open.take() {
            let mut keep = true;
            egui::Window::new(t("Open Scene", "Sahne Aç"))
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut path).desired_width(460.0));
                    let found = find_scenes(&self.settings.projects_dir);
                    if !found.is_empty() {
                        egui::ScrollArea::vertical()
                            .max_height(220.0)
                            .show(ui, |ui| {
                                for f in found {
                                    if ui
                                        .selectable_label(
                                            path == f.display().to_string(),
                                            f.display().to_string(),
                                        )
                                        .clicked()
                                    {
                                        path = f.display().to_string();
                                    }
                                }
                            });
                    }
                    ui.horizontal(|ui| {
                        if ui.button(t("Open", "Aç")).clicked() {
                            self.open_scene(PathBuf::from(path.trim()));
                            keep = false;
                        }
                        if ui.button(t("Cancel", "İptal")).clicked() {
                            keep = false;
                        }
                    });
                });
            if keep {
                self.dialogs.open = Some(path);
            }
        }
        if let Some(mut path) = self.dialogs.save_as.take() {
            let mut keep = true;
            egui::Window::new(t("Save Scene As", "Sahneyi Farklı Kaydet"))
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut path).desired_width(460.0));
                    ui.horizontal(|ui| {
                        if ui.button(t("Save", "Kaydet")).clicked() {
                            let mut p = PathBuf::from(path.trim());
                            if p.extension().is_none() {
                                p.set_extension("json");
                            }
                            self.save_to(p);
                            keep = false;
                        }
                        if ui.button(t("Cancel", "İptal")).clicked() {
                            keep = false;
                        }
                    });
                });
            if keep {
                self.dialogs.save_as = Some(path);
            }
        }
        if let Some(mut path) = self.dialogs.build.take() {
            let mut keep = true;
            egui::Window::new(t("Build Game", "Oyunu Derle")).collapsible(false).show(ctx, |ui| {
                ui.label(t("Output folder — Forge copies its player runtime, the scene and textures there.", "Çıktı klasörü — Forge oynatıcıyı, sahneyi ve dokuları buraya kopyalar."));
                ui.add(egui::TextEdit::singleline(&mut path).desired_width(460.0));
                ui.horizontal(|ui| {
                    if ui.button(t("🔨 Build", "🔨 Derle")).clicked() {
                        self.build_game(PathBuf::from(path.trim()));
                        keep = false;
                    }
                    if ui.button(t("Cancel", "İptal")).clicked() {
                        keep = false;
                    }
                });
            });
            if keep {
                self.dialogs.build = Some(path);
            }
        }
        if let Some((id, mut name)) = self.dialogs.prefab_name.take() {
            let mut keep = true;
            egui::Window::new(t("Create Prefab", "Prefab Oluştur"))
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.text_edit_singleline(&mut name);
                    ui.horizontal(|ui| {
                        if ui.button(t("Save", "Kaydet")).clicked() {
                            self.save_prefab(id, &name);
                            keep = false;
                        }
                        if ui.button(t("Cancel", "İptal")).clicked() {
                            keep = false;
                        }
                    });
                });
            if keep {
                self.dialogs.prefab_name = Some((id, name));
            }
        }
        if self.dialogs.settings {
            let mut open = true;
            egui::Window::new(t("AI Settings", "AI Ayarları"))
                .open(&mut open)
                .default_width(540.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(560.0)
                        .show(ui, |ui| self.ai_settings_ui(ui));
                });
            self.dialogs.settings = open;
        }
        if self.dialogs.scripting_help {
            let mut open = true;
            egui::Window::new(t("Scripting reference", "Betik başvurusu"))
                .open(&mut open)
                .default_width(560.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(480.0)
                        .show(ui, |ui| {
                            ui.label(RichText::new(tools::SCRIPT_DOCS).monospace().size(12.0));
                        });
                });
            self.dialogs.scripting_help = open;
        }
        if self.dialogs.about {
            let mut open = true;
            egui::Window::new(t("About Forge", "Forge hakkında")).open(&mut open).collapsible(false).show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(96.0, 96.0), egui::Sense::hover());
                    theme::draw_logo(ui.painter(), r);
                    ui.label(RichText::new("Forge").size(24.0).strong());
                    ui.label(format!("v{} · build {} · {}", env!("CARGO_PKG_VERSION"), crate::updater::BUILD, crate::updater::COMMIT));
                    ui.label(t("Open-source, AI-native game engine in Rust", "Rust ile yazılmış açık kaynak, AI destekli oyun motoru"));
                    ui.small("egui · OpenGL · Rhai · llama.cpp · OpenRouter · TencentDB Agent Memory");
                });
            });
            self.dialogs.about = open;
        }
    }

    pub fn ai_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(t("API keys", "API anahtarları")).strong());
        egui::Grid::new("keys").num_columns(2).show(ui, |ui| {
            ui.label("OpenRouter");
            ui.add(
                egui::TextEdit::singleline(&mut self.secrets.openrouter_key)
                    .password(true)
                    .desired_width(340.0),
            );
            ui.end_row();
            ui.label(t("Hugging Face (opt.)", "Hugging Face (ops.)"));
            ui.add(
                egui::TextEdit::singleline(&mut self.secrets.hf_token)
                    .password(true)
                    .desired_width(340.0),
            );
            ui.end_row();
        });
        ui.horizontal(|ui| {
            if ui
                .button(t("💾 Save keys", "💾 Anahtarları kaydet"))
                .clicked()
            {
                let msg = match self.secrets.save() {
                    Ok(()) => format!("Keys saved to {}", crate::config::env_file().display()),
                    Err(e) => format!("Could not save keys: {e}"),
                };
                self.console.push((LogLevel::Info, msg));
            }
            ui.small(crate::config::env_file().display().to_string());
        });
        ui.separator();
        ui.label(RichText::new(t("Agents", "Ajanlar")).strong());
        let s = &mut self.settings;
        let mut options: Vec<(String, String)> = vec![(
            String::new(),
            t("(same as chat model)", "(sohbet modeliyle aynı)").into(),
        )];
        options.extend(
            crate::ai::models::CLOUD_MODELS
                .iter()
                .map(|m| (m.id.to_string(), m.name.to_string())),
        );
        options.extend(s.custom_models.iter().map(|m| (m.clone(), m.clone())));
        options.extend(
            self.library
                .installed_models()
                .into_iter()
                .map(|m| (m.clone(), crate::ai::models::display_name(&m))),
        );
        options.extend(
            self.library
                .ollama_models()
                .into_iter()
                .map(|m| (m.clone(), crate::ai::models::display_name(&m))),
        );
        egui::Grid::new("agents")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                for (label, value) in [
                    ("🗺 Planner", &mut s.planner_model),
                    ("🔨 Builder", &mut s.builder_model),
                    ("⌨ Coder", &mut s.coder_model),
                    ("🔭 Explore", &mut s.router_model),
                    ("🔍 Reviewer", &mut s.reviewer_model),
                ] {
                    ui.label(label);
                    let shown = options
                        .iter()
                        .find(|(id, _)| id == value)
                        .map(|(_, n)| n.clone())
                        .unwrap_or_else(|| value.clone());
                    egui::ComboBox::from_id_salt(label)
                        .selected_text(shown)
                        .width(300.0)
                        .show_ui(ui, |ui| {
                            for (id, n) in &options {
                                ui.selectable_value(value, id.clone(), n);
                            }
                        });
                    ui.end_row();
                }
            });
        ui.checkbox(
            &mut s.use_planner,
            t(
                "Planner agent in Planning mode",
                "Planlama modunda Planlayıcı ajan",
            ),
        );
        ui.checkbox(
            &mut s.use_reviewer,
            t(
                "Reviewer agent (checks result, one fix round)",
                "Denetçi ajan (sonucu kontrol eder, bir düzeltme turu)",
            ),
        );
        ui.horizontal(|ui| {
            ui.label(t("Max builder steps", "Maks. kurucu adımı"));
            ui.add(egui::DragValue::new(&mut s.max_agent_steps).range(1..=30));
            ui.label(t("Temperature", "Sıcaklık"));
            ui.add(egui::Slider::new(&mut s.temperature, 0.0..=1.5));
        });
        ui.label(
            RichText::new(t(
                "Permissions: deleting entities, clearing the scene and writing files ask first.",
                "İzinler: nesne silme, sahneyi temizleme ve dosya yazma önce sorulur.",
            ))
            .small()
            .weak(),
        );
        if !self.always_allow.is_empty()
            && ui
                .small_button(format!(
                    "{} ({})",
                    t("Reset 'always allow'", "'Her zaman izin ver'i sıfırla"),
                    self.always_allow.join(", ")
                ))
                .clicked()
        {
            self.always_allow.clear();
        }
        ui.separator();
        panels::memory_settings(ui, &mut self.settings, &self.memory_status);
        if ui.button(t("Save settings", "Ayarları kaydet")).clicked() {
            self.settings.save();
        }
    }
}

impl TabViewer for Editor {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Scene => self.viewport_ui(ui, false),
            Tab::Game => self.viewport_ui(ui, true),
            Tab::Hierarchy => self.hierarchy_ui(ui),
            Tab::Library => {
                let current = self.settings.selected_model.clone();
                let actions = self
                    .library
                    .ui(ui, &mut self.settings, &self.secrets, &current);
                let ctx = ui.ctx().clone();
                for a in actions {
                    match a {
                        LibAction::UseModel(m) => self.select_model(m, &ctx),
                        LibAction::Log(m) => self.log(LogLevel::Info, m),
                    }
                }
            }
            Tab::Inspector => {
                if self.runtime.is_some() {
                    ui.label(
                        RichText::new(t(
                            "Playing — edits are disabled. Stop (■) to edit.",
                            "Oynatılıyor — düzenleme kapalı. Düzenlemek için ■.",
                        ))
                        .color(theme::GOLD),
                    );
                } else {
                    self.inspector_ui(ui);
                }
            }
            Tab::Lighting => self.lighting_ui(ui),
            Tab::ProjectSettings => self.project_settings_ui(ui),
            Tab::Content => self.content_ui(ui),
            Tab::Console => self.console_ui(ui),
            Tab::Script => self.script_ui(ui),
        }
    }

    fn closeable(&mut self, _tab: &mut Tab) -> bool {
        true
    }

    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        match tab {
            Tab::Scene
            | Tab::Game
            | Tab::Library
            | Tab::Content
            | Tab::Console
            | Tab::Script
            | Tab::Hierarchy => [false, false],
            _ => [false, true],
        }
    }

    fn clear_background(&self, tab: &Tab) -> bool {
        !matches!(tab, Tab::Scene | Tab::Game)
    }
}

pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn find_scenes(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Ok(rd2) = std::fs::read_dir(&p) {
                    out.extend(
                        rd2.flatten()
                            .map(|e| e.path())
                            .filter(|p| p.to_string_lossy().ends_with(".scene.json")),
                    );
                }
            }
        }
    }
    out.sort();
    out
}

impl eframe::App for ForgeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        let ed = &mut self.ed;
        ed.fps = ed.fps * 0.95 + (1.0 / dt.max(1e-4)) * 0.05;

        // ---------------------------------------------------------------- first-run wizard
        if self.screen == Screen::Wizard {
            if let Some(w) = &mut self.wizard {
                if let Some(res) = w.ui(
                    ctx,
                    &mut ed.settings,
                    &mut ed.secrets,
                    &mut ed.library,
                    &mut self.updater,
                ) {
                    self.wizard = None;
                    ed.library.refresh_installed(&ed.settings);
                    open_or_create(ed, res.scene_path, res.template);
                    ed.log(
                        LogLevel::Info,
                        t(
                            "Setup complete — welcome to Forge!",
                            "Kurulum tamamlandı — Forge'a hoş geldin!",
                        )
                        .into(),
                    );
                    self.screen = Screen::Editor;
                }
            }
            return;
        }

        // ---------------------------------------------------------------- start screen
        if self.screen == Screen::Start {
            if let Some(s) = &mut self.start {
                if let Some(a) = s.ui(ctx, &mut ed.settings, &mut self.updater) {
                    match a {
                        StartAction::Open(p) => ed.open_scene(p),
                        StartAction::New { path, template } => open_or_create(ed, path, template),
                    }
                    self.start = None;
                    self.screen = Screen::Editor;
                }
            }
            return;
        }

        // ---------------------------------------------------------------- standalone player
        if ed.player_mode {
            ed.tick_runtime(ctx, dt);
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| ed.viewport_ui(ui, true));
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }

        // While the editor is open, look for new GitHub releases every 30 minutes.
        if self.last_update_check.elapsed().as_secs() > 30 * 60 {
            self.last_update_check = Instant::now();
            if matches!(
                self.updater.state(),
                crate::updater::UpdateState::UpToDate
                    | crate::updater::UpdateState::Failed(_)
                    | crate::updater::UpdateState::Idle
            ) {
                self.updater.check(ctx);
            }
        }

        // Agent events → execute tools on the scene (destructive ones wait for approval).
        let mut chat = std::mem::take(&mut ed.chat);
        let allow = ed.always_allow.clone();
        chat.poll(
            |call| ed.exec_tool(call),
            |call| tools::DESTRUCTIVE.contains(&call.name.as_str()) && !allow.contains(&call.name),
        );
        ed.chat = chat;

        ed.tick_runtime(ctx, dt);

        // Shortcuts.
        let typing = ctx.wants_keyboard_input();
        let (save, undo, redo, dup, del, play, focus_ai, w, e, r) = ctx.input(|i| {
            let c = i.modifiers.command;
            (
                c && i.key_pressed(egui::Key::S),
                c && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                c && (i.key_pressed(egui::Key::Y)
                    || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
                c && i.key_pressed(egui::Key::D),
                i.key_pressed(egui::Key::Delete),
                c && i.key_pressed(egui::Key::P),
                c && i.key_pressed(egui::Key::L),
                !c && i.key_pressed(egui::Key::W) && !i.pointer.secondary_down(),
                !c && i.key_pressed(egui::Key::E) && !i.pointer.secondary_down(),
                !c && i.key_pressed(egui::Key::R) && !i.pointer.secondary_down(),
            )
        });
        if save {
            ed.save();
        }
        if play {
            if ed.runtime.is_some() {
                ed.stop()
            } else {
                ed.play()
            }
        }
        if focus_ai {
            ed.chat.focus();
        }
        if !typing && ed.runtime.is_none() {
            if undo {
                ed.undo();
            }
            if redo {
                ed.redo();
            }
            if dup {
                ed.duplicate_selected();
            }
            if del {
                ed.delete_selected();
            }
            if w {
                ed.tool = Tool::Move;
            }
            if e {
                ed.tool = Tool::Rotate;
            }
            if r {
                ed.tool = Tool::Scale;
            }
        }

        let title = format!(
            "Forge — {}{}",
            ed.scene_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ed.scene.name.clone()),
            if ed.dirty { " *" } else { "" }
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        egui::TopBottomPanel::top("menu")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG0)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ctx, |ui| {
                ed.menu_bar(ui, ctx);
            });

        // Status bar.
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG0)
                    .inner_margin(egui::Margin::symmetric(10, 3)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("🎬 {}", ed.scene.name))
                            .size(11.0)
                            .weak(),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} {}",
                            ed.scene.entities.len(),
                            t("objects", "nesne")
                        ))
                        .size(11.0)
                        .weak(),
                    );
                    if let Some(sel) = ed.selected.and_then(|s| ed.scene.get(s)) {
                        ui.label(
                            RichText::new(format!("› {}", sel.name))
                                .size(11.0)
                                .color(ACCENT),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(rel) = self.updater.available() {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(format!("⬆ Build {}", rel.build))
                                            .size(11.0)
                                            .color(theme::GOLD),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.updater.dismissed = false;
                                self.start = Some(StartScreen::new(&ed.settings));
                                self.screen = Screen::Start;
                            }
                        }
                        let mem = match ed.settings.memory.backend {
                            crate::ai::memory::MemoryBackend::Off => {
                                t("memory off", "hafıza kapalı")
                            }
                            crate::ai::memory::MemoryBackend::Builtin => {
                                t("💭 built-in memory", "💭 yerleşik hafıza")
                            }
                            crate::ai::memory::MemoryBackend::TencentDb => {
                                "💭 TencentDB Agent Memory"
                            }
                        };
                        ui.label(RichText::new(mem).size(11.0).weak());
                        ui.label(
                            RichText::new(crate::ai::models::display_name(
                                &ed.settings.selected_model,
                            ))
                            .size(11.0)
                            .color(theme::GOLD),
                        );
                        ui.label(
                            RichText::new(format!("{:.1} ms", ed.frame_ms))
                                .size(11.0)
                                .weak(),
                        );
                    });
                });
            });

        // Right: agent panel (Antigravity-style, outside the dock).
        egui::SidePanel::right("agent")
            .default_width(370.0)
            .width_range(300.0..=680.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG1)
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(ctx, |ui| {
                let mut local = ed.library.installed_models();
                local.extend(ed.library.ollama_models());
                let current = ed.settings.selected_model.clone();
                let names: Vec<String> = ed.scene.entities.iter().map(|e| e.name.clone()).collect();
                let project = ed
                    .scene_path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| ed.scene.name.clone());
                let reqs = ed.chat.ui(ui, &current, &local, &names, &project);
                ed.handle_chat_requests(reqs, ctx);
            });

        // Dock layout requests.
        if ed.reset_layout {
            self.dock = default_layout();
            ed.reset_layout = false;
        }
        if let Some(tab) = ed.open_tab.take() {
            if let Some(found) = self.dock.find_tab(&tab) {
                self.dock.set_active_tab(found);
            } else {
                self.dock.push_to_focused_leaf(tab);
            }
        }
        if let Some(tab) = ed.focus_tab.take() {
            if let Some(found) = self.dock.find_tab(&tab) {
                self.dock.set_active_tab(found);
            }
        }

        let mut style = Style::from_egui(ctx.style().as_ref());
        style.tab_bar.bg_fill = theme::BG0;
        style.tab.active.bg_fill = theme::BG1;
        style.tab.focused.bg_fill = theme::BG1;
        style.tab.inactive.bg_fill = theme::BG0;
        style.tab.hovered.bg_fill = theme::BG2;
        style.tab.active.text_color = theme::BONE;
        style.tab.focused.text_color = theme::BONE;
        style.tab.inactive.text_color = Color32::from_gray(140);
        style.separator.color_idle = theme::BG0;
        style.separator.width = 3.0;
        DockArea::new(&mut self.dock)
            .style(style)
            .show_add_buttons(false)
            .show_leaf_collapse_buttons(false)
            .show(ctx, ed);

        ed.dialogs_ui(ctx);
        ed.frame_ms = ed.frame_ms * 0.9 + now.elapsed().as_secs_f32() * 1000.0 * 0.1;
        if ed.chat.busy() || !ed.library.downloads.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.ed.settings.agent_web = self.ed.chat.web;
        self.ed.settings.agent_mode = self.ed.chat.mode;
        self.ed.settings.save();
        self.ed.library.stop_llama();
    }
}

fn open_or_create(ed: &mut Editor, path: PathBuf, template: Template) {
    if path.is_file() {
        ed.open_scene(path);
    } else {
        ed.scene = template.build();
        ed.scene.name = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Main".into());
        ed.save_to(path);
    }
}
