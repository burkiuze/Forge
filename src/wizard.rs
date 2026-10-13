//! First-run setup wizard (Kurulum Sihirbazı).

use crate::ai::models::CLOUD_MODELS;
use crate::config::{Secrets, Settings};
use crate::i18n::{self, t, Lang};
use crate::library::{start_download, DlStatus, Download, Library};
use crate::templates::Template;
use crate::theme;
use crate::updater::Updater;
use egui::RichText;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn steps() -> [&'static str; 5] {
    [
        t("Welcome", "Hoş geldin"),
        t("Cloud AI", "Bulut AI"),
        t("Local AI", "Yerel AI"),
        t("Project", "Proje"),
        t("Done", "Bitti"),
    ]
}

const STARTER_MODELS: &[(&str, &str, &str, &str)] = &[
    (
        "Qwen2.5 Coder 7B",
        "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF",
        "qwen2.5-coder-7b-instruct-q4_k_m.gguf",
        "≈4.7 GB · RAM 8 GB · code & scripts",
    ),
    (
        "Llama 3.2 3B",
        "bartowski/Llama-3.2-3B-Instruct-GGUF",
        "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        "≈2.0 GB · RAM 4 GB · fast",
    ),
];

pub struct Wizard {
    step: usize,
    key: String,
    show_key: bool,
    key_check: Arc<Mutex<Option<Result<String, String>>>>,
    model: String,
    models_dir: String,
    project_name: String,
    project_dir: String,
    template: Template,
    downloads: Vec<Arc<Download>>,
}

pub struct WizardResult {
    pub scene_path: PathBuf,
    pub template: Template,
}

impl Wizard {
    pub fn new(settings: &Settings, secrets: &Secrets) -> Self {
        Self {
            step: 0,
            key: secrets.openrouter_key.clone(),
            show_key: false,
            key_check: Arc::new(Mutex::new(None)),
            model: settings.selected_model.clone(),
            models_dir: settings.models_dir.display().to_string(),
            project_name: "My First Game".into(),
            project_dir: settings.projects_dir.display().to_string(),
            template: Template::Platformer,
            downloads: vec![],
        }
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        secrets: &mut Secrets,
        library: &mut Library,
        updater: &mut Updater,
    ) -> Option<WizardResult> {
        let mut result = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG0))
            .show(ctx, |ui| {
                let avail = ui.available_rect_before_wrap();
                let w = 720.0f32.min(avail.width() - 32.0);
                let h = 520.0f32.min(avail.height() - 32.0);
                let card = egui::Rect::from_center_size(avail.center(), egui::vec2(w, h));
                ui.painter().rect_filled(card, 12.0, theme::BG1);
                ui.painter().rect_stroke(
                    card,
                    12.0,
                    egui::Stroke::new(1.0_f32, theme::BG3),
                    egui::StrokeKind::Inside,
                );
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(card.shrink(24.0)));
                let ui = &mut child;

                // Stepper
                ui.horizontal(|ui| {
                    let steps = steps();
                    for (i, s) in steps.iter().enumerate() {
                        let c = if i == self.step {
                            theme::ACCENT
                        } else if i < self.step {
                            theme::BONE
                        } else {
                            egui::Color32::from_gray(110)
                        };
                        ui.label(RichText::new(format!("{} {}", i + 1, s)).color(c).strong());
                        if i + 1 < steps.len() {
                            ui.label(RichText::new("›").color(theme::BG3));
                        }
                    }
                });
                ui.separator();
                ui.add_space(8.0);

                let body_h = ui.available_height() - 44.0;
                egui::ScrollArea::vertical()
                    .max_height(body_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| match self.step {
                        0 => self.welcome(ui, settings, updater),
                        1 => self.cloud(ui),
                        2 => self.local(ui, settings, library),
                        3 => self.project(ui),
                        _ => self.finish(ui),
                    });

                ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                    ui.horizontal(|ui| {
                        let last = self.step + 1 == steps().len();
                        let label = if last {
                            t("🚀 Launch Forge", "🚀 Forge'u Başlat")
                        } else {
                            t("Next ›", "İleri ›")
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(label).color(egui::Color32::WHITE).strong(),
                                )
                                .fill(theme::ACCENT)
                                .min_size(egui::vec2(140.0, 30.0)),
                            )
                            .clicked()
                        {
                            if last {
                                result = Some(self.apply(settings, secrets));
                            } else {
                                self.step += 1;
                            }
                        }
                        if self.step > 0
                            && ui
                                .add(
                                    egui::Button::new(t("‹ Back", "‹ Geri"))
                                        .min_size(egui::vec2(80.0, 30.0)),
                                )
                                .clicked()
                        {
                            self.step -= 1;
                        }
                        if self.step < 4
                            && ui
                                .button(t("Skip", "Atla"))
                                .on_hover_text(t(
                                    "Continue with defaults",
                                    "Varsayılanlarla devam et",
                                ))
                                .clicked()
                        {
                            self.step = 4;
                        }
                    });
                });
            });
        result
    }

    fn welcome(&mut self, ui: &mut egui::Ui, settings: &mut Settings, updater: &mut Updater) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Language / Dil").strong());
            for l in [Lang::En, Lang::Tr] {
                if ui
                    .selectable_label(settings.lang == l, RichText::new(l.label()).size(14.0))
                    .clicked()
                {
                    settings.lang = l;
                    i18n::set(l);
                }
            }
        });
        if let Some(rel) = updater.available().filter(|_| !updater.dismissed) {
            egui::Frame::new()
                .fill(theme::BG2)
                .stroke(egui::Stroke::new(1.0_f32, theme::GOLD))
                .corner_radius(8)
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(format!(
                            "{} — Build {}",
                            t("A Forge update is available", "Forge güncellemesi mevcut"),
                            rel.build
                        ))
                        .color(theme::GOLD)
                        .strong(),
                    );
                    ui.label(t("Do you want to update now?", "Şimdi güncellensin mi?"));
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(t("Update", "Güncelle"))
                                        .color(egui::Color32::WHITE),
                                )
                                .fill(theme::ACCENT),
                            )
                            .clicked()
                        {
                            updater.install(ui.ctx());
                        }
                        if ui.button(t("Later", "Daha sonra")).clicked() {
                            updater.dismissed = true;
                        }
                    });
                });
        }
        match updater.state() {
            crate::updater::UpdateState::Downloading(p) => {
                ui.add(
                    egui::ProgressBar::new(p)
                        .show_percentage()
                        .text(t("Downloading update…", "Güncelleme indiriliyor…")),
                );
            }
            crate::updater::UpdateState::ReadyToRestart => {
                if ui
                    .button(t("Restart now", "Şimdi yeniden başlat"))
                    .clicked()
                {
                    updater.restart(ui.ctx());
                }
            }
            _ => {}
        }
        ui.vertical_centered(|ui| {
            ui.add_space(10.0);
            let (r, _) = ui.allocate_exact_size(egui::vec2(120.0, 120.0), egui::Sense::hover());
            theme::draw_logo(ui.painter(), r);
            ui.add_space(6.0);
            ui.label(
                RichText::new(t("Welcome to Forge", "Forge'a hoş geldin"))
                    .size(28.0)
                    .strong(),
            );
            ui.label(
                RichText::new(t(
                    "Open-source, AI-native game engine written in Rust",
                    "AI destekli, Rust ile yazılmış açık kaynak oyun motoru",
                ))
                .size(15.0)
                .color(theme::GOLD),
            );
            ui.add_space(14.0);
        });
        for (icon, t, d) in [
            ("✨", t("Forge AI agents", "Forge AI ajanları"), t("Planner, Builder, Coder, Explore and Reviewer agents build your scene, with web search and long-term memory.", "Planner, Builder, Coder, Explore ve Reviewer ajanları web araması ve uzun süreli hafızayla sahneni kurar.")),
            ("☁", t("Cloud models", "Bulut modelleri"), "Auto, Gemma 4, Mercury Decide, North Mini Code, Inkling Small…"),
            ("💻", t("Local models", "Yerel modeller"), t("400+ GGUF models with size and hardware requirements, run offline with llama.cpp.", "Boyut ve donanım gereksinimiyle 400+ GGUF model, llama.cpp ile internetsiz çalıştır.")),
            ("▶", t("Play instantly", "Anında oyna"), t("Rhai scripts, physics, shadows and one-click Play mode.", "Rhai betikleri, fizik, gölgeler ve tek tıkla Play modu.")),
        ] {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(18.0).color(theme::ACCENT));
                ui.vertical(|ui| {
                    ui.label(RichText::new(t).strong());
                    ui.small(d);
                });
            });
            ui.add_space(4.0);
        }
    }

    fn cloud(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new(t("OpenRouter API key", "OpenRouter API anahtarı"))
                .size(18.0)
                .strong(),
        );
        ui.small(t("Your key is stored only on this computer, in the .env file of Forge's data folder (OPENROUTER_API_KEY). A .env from the Forge.env repo is read automatically too.", "Anahtarın yalnızca bu bilgisayarda saklanır: Forge veri klasöründeki .env dosyası (OPENROUTER_API_KEY). Forge.env deposundaki .env de otomatik okunur."));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.key)
                    .password(!self.show_key)
                    .hint_text("sk-or-v1-…")
                    .desired_width(420.0),
            );
            ui.checkbox(&mut self.show_key, t("Show", "Göster"));
            if ui.button(t("Test", "Test et")).clicked() {
                let key = self.key.clone();
                let slot = self.key_check.clone();
                let ctx = ui.ctx().clone();
                *slot.lock().unwrap() = None;
                std::thread::spawn(move || {
                    let r = crate::ai::client::check_openrouter_key(&key);
                    *slot.lock().unwrap() = Some(r);
                    ctx.request_repaint();
                });
            }
        });
        match &*self.key_check.lock().unwrap() {
            Some(Ok(m)) => {
                ui.colored_label(egui::Color32::from_rgb(120, 220, 140), format!("✔ {m}"));
            }
            Some(Err(e)) => {
                ui.colored_label(egui::Color32::LIGHT_RED, format!("✖ {e}"));
            }
            None => {}
        }
        ui.hyperlink_to(
            t(
                "Get a key: openrouter.ai/keys",
                "Anahtar al: openrouter.ai/keys",
            ),
            "https://openrouter.ai/keys",
        );
        ui.add_space(10.0);
        ui.label(RichText::new(t("Default model", "Varsayılan model")).strong());
        for m in CLOUD_MODELS {
            ui.radio_value(
                &mut self.model,
                m.id.to_string(),
                format!("{} — {}", m.name, m.blurb),
            );
        }
    }

    fn local(&mut self, ui: &mut egui::Ui, settings: &mut Settings, library: &mut Library) {
        ui.label(
            RichText::new(t("Local AI (optional)", "Yerel AI (isteğe bağlı)"))
                .size(18.0)
                .strong(),
        );
        ui.small(t("Download the llama.cpp runtime and a GGUF model to work offline. You can also do this later from the Library.", "İnternetsiz çalışmak için llama.cpp çalışma zamanını ve bir GGUF model indir. Daha sonra Kütüphane'den de yapabilirsin."));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(t("Models folder", "Model klasörü"));
            ui.add(egui::TextEdit::singleline(&mut self.models_dir).desired_width(440.0));
        });
        settings.models_dir = PathBuf::from(&self.models_dir);
        ui.add_space(8.0);
        let installed = Library::find_llama_server(settings).is_some();
        ui.horizontal(|ui| {
            ui.label(RichText::new(t("llama.cpp runtime", "llama.cpp çalışma zamanı")).strong());
            if installed {
                ui.colored_label(
                    egui::Color32::from_rgb(120, 220, 140),
                    t("✔ installed", "✔ kurulu"),
                );
            } else if ui
                .button(t("⬇ Download & install", "⬇ İndir & kur"))
                .clicked()
            {
                library.install_runtime(settings, ui.ctx());
            }
        });
        library.runtime_progress_ui(ui);
        ui.add_space(8.0);
        ui.label(RichText::new(t("Starter model", "Başlangıç modeli")).strong());
        for (name, repo, file, info) in STARTER_MODELS {
            let path = settings.models_dir.join(file);
            ui.horizontal(|ui| {
                ui.label(format!("{name}  ·  {info}"));
                if path.is_file() {
                    ui.colored_label(egui::Color32::from_rgb(120, 220, 140), "✔");
                } else if !self.downloads.iter().any(|d| d.label == *file)
                    && ui.small_button(t("⬇ Download", "⬇ İndir")).clicked()
                {
                    let url = format!("https://huggingface.co/{repo}/resolve/main/{file}");
                    self.downloads.push(start_download(
                        file.to_string(),
                        url,
                        path.clone(),
                        String::new(),
                        ui.ctx().clone(),
                    ));
                }
            });
        }
        for d in &self.downloads {
            match d.status.lock().unwrap().clone() {
                DlStatus::Running => {
                    ui.add(
                        egui::ProgressBar::new(d.progress())
                            .show_percentage()
                            .text(&d.label),
                    );
                }
                DlStatus::Failed(e) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("{}: {e}", d.label));
                }
                DlStatus::Finished => {
                    ui.small(format!("✔ {}", d.label));
                }
            }
        }
        // Hand still-running downloads to the library so they show up there too.
        let new: Vec<_> = self
            .downloads
            .iter()
            .filter(|d| !library.downloads.iter().any(|x| Arc::ptr_eq(x, d)))
            .cloned()
            .collect();
        library.downloads.extend(new);
    }

    fn project(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new(t("Your first project", "İlk projen"))
                .size(18.0)
                .strong(),
        );
        egui::Grid::new("proj")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label(t("Project name", "Proje adı"));
                ui.text_edit_singleline(&mut self.project_name);
                ui.end_row();
                ui.label(t("Location", "Konum"));
                ui.add(egui::TextEdit::singleline(&mut self.project_dir).desired_width(420.0));
                ui.end_row();
            });
        ui.add_space(8.0);
        ui.label(RichText::new(t("Template", "Şablon")).strong());
        for t in Template::ALL {
            ui.radio_value(&mut self.template, t, t.label());
        }
    }

    fn finish(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(80.0, 80.0), egui::Sense::hover());
            theme::draw_logo(ui.painter(), r);
            ui.label(
                RichText::new(t("All set!", "Her şey hazır!"))
                    .size(24.0)
                    .strong(),
            );
        });
        ui.add_space(10.0);
        let key = if self.key.trim().is_empty() {
            t("— (can be added later)", "— (sonra eklenebilir)").to_string()
        } else {
            t("✔ will be saved", "✔ kaydedilecek").to_string()
        };
        egui::Grid::new("sum")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label(t("OpenRouter key", "OpenRouter anahtarı"));
                ui.label(key);
                ui.end_row();
                ui.label(t("Default model", "Varsayılan model"));
                ui.label(crate::ai::models::display_name(&self.model));
                ui.end_row();
                ui.label(t("Project", "Proje"));
                ui.label(self.project_path().display().to_string());
                ui.end_row();
                ui.label(t("Template", "Şablon"));
                ui.label(self.template.label());
                ui.end_row();
            });
        ui.add_space(10.0);
        ui.small(t("Tip: ask the Agent panel on the right \"Build a maze out of red cubes\". Press ▶ to play, ■ to go back to editing.", "İpucu: Sağdaki Ajan paneline \"Kırmızı küplerden bir labirent yap\" yazmayı dene. ▶ ile oyna, ■ ile düzenlemeye dön."));
    }

    fn project_path(&self) -> PathBuf {
        let safe: String = self
            .project_name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        PathBuf::from(&self.project_dir)
            .join(safe.trim())
            .join("main.scene.json")
    }

    fn apply(&mut self, settings: &mut Settings, secrets: &mut Secrets) -> WizardResult {
        secrets.openrouter_key = self.key.trim().to_string();
        let _ = secrets.save();
        settings.selected_model = self.model.clone();
        settings.models_dir = PathBuf::from(&self.models_dir);
        settings.projects_dir = PathBuf::from(&self.project_dir);
        settings.setup_done = true;
        crate::assets::ensure_project(
            self.project_path()
                .parent()
                .unwrap_or(std::path::Path::new(".")),
        );
        let path = self.project_path();
        settings.last_scene = Some(path.clone());
        settings.save();
        WizardResult {
            scene_path: path,
            template: self.template,
        }
    }
}
