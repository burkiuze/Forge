//! Start screen shown on every launch: recent projects, new project, and the update prompt.

use crate::config::Settings;
use crate::i18n::{self, t, Lang};
use crate::templates::Template;
use crate::theme;
use crate::updater::{UpdateState, Updater, BUILD, COMMIT};
use egui::{Color32, RichText};
use std::path::PathBuf;

pub enum StartAction {
    Open(PathBuf),
    New { path: PathBuf, template: Template },
}

pub struct StartScreen {
    new_name: String,
    template: Template,
    open_path: String,
    recent: Vec<(PathBuf, String, u64)>,
}

impl StartScreen {
    pub fn new(settings: &Settings) -> Self {
        let mut recent = vec![];
        if let Ok(rd) = std::fs::read_dir(&settings.projects_dir) {
            for e in rd.flatten() {
                let dir = e.path();
                if !dir.is_dir() {
                    continue;
                }
                if let Ok(rd2) = std::fs::read_dir(&dir) {
                    for f in rd2.flatten() {
                        let p = f.path();
                        if p.to_string_lossy().ends_with(".scene.json") {
                            let m = f
                                .metadata()
                                .and_then(|m| m.modified())
                                .ok()
                                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            recent.push((
                                p,
                                dir.file_name().unwrap().to_string_lossy().to_string(),
                                m,
                            ));
                        }
                    }
                }
            }
        }
        recent.sort_by(|a, b| b.2.cmp(&a.2));
        Self {
            new_name: "New Game".into(),
            template: Template::Platformer,
            open_path: String::new(),
            recent,
        }
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        updater: &mut Updater,
    ) -> Option<StartAction> {
        let mut action = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG0).inner_margin(28))
            .show(ctx, |ui| {
                // Top bar: language + update status.
                ui.horizontal(|ui| {
                    let (r, _) =
                        ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                    theme::draw_logo(ui.painter(), r);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Forge").size(22.0).strong());
                        let build = if BUILD > 0 {
                            format!("Build {BUILD} · {COMMIT}")
                        } else {
                            format!("dev · {COMMIT}")
                        };
                        ui.label(
                            RichText::new(format!("v{} · {build}", env!("CARGO_PKG_VERSION")))
                                .size(11.0)
                                .weak(),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for l in [Lang::Tr, Lang::En] {
                            if ui.selectable_label(settings.lang == l, l.label()).clicked() {
                                settings.lang = l;
                                i18n::set(l);
                                settings.save();
                            }
                        }
                        ui.add_space(12.0);
                        match updater.state() {
                            UpdateState::Checking => {
                                ui.label(
                                    RichText::new(t(
                                        "Checking for updates…",
                                        "Güncellemeler denetleniyor…",
                                    ))
                                    .size(11.5)
                                    .weak(),
                                );
                                ui.spinner();
                            }
                            UpdateState::UpToDate => {
                                ui.label(
                                    RichText::new(t("✔ Forge is up to date", "✔ Forge güncel"))
                                        .size(11.5)
                                        .color(Color32::from_rgb(120, 210, 140)),
                                );
                            }
                            UpdateState::Failed(e) => {
                                ui.label(
                                    RichText::new(format!(
                                        "{} ({e})",
                                        t("Update check failed", "Güncelleme denetlenemedi")
                                    ))
                                    .size(11.0)
                                    .weak(),
                                );
                                if ui.small_button("⟳").clicked() {
                                    updater.check(ui.ctx());
                                }
                            }
                            UpdateState::Available if updater.dismissed => {
                                if ui
                                    .button(
                                        RichText::new(t("⬆ Update available", "⬆ Güncelleme var"))
                                            .color(theme::GOLD),
                                    )
                                    .clicked()
                                {
                                    updater.dismissed = false;
                                }
                            }
                            _ => {}
                        }
                    });
                });
                ui.add_space(18.0);

                ui.columns(2, |cols| {
                    // Left: new / open.
                    let ui = &mut cols[0];
                    ui.label(
                        RichText::new(t("New project", "Yeni proje"))
                            .size(16.0)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    egui::Frame::group(ui.style())
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(t("Project name", "Proje adı"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_name)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.add_space(6.0);
                            ui.label(t("Template", "Şablon"));
                            for tp in Template::ALL {
                                ui.radio_value(&mut self.template, tp, tp.label());
                            }
                            ui.add_space(8.0);
                            let btn = egui::Button::new(
                                RichText::new(t("+ Create project", "+ Proje oluştur"))
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(theme::ACCENT)
                            .min_size(egui::vec2(160.0, 30.0));
                            if ui.add(btn).clicked() && !self.new_name.trim().is_empty() {
                                let safe: String = self
                                    .new_name
                                    .chars()
                                    .map(|c| {
                                        if c.is_alphanumeric() || " -_".contains(c) {
                                            c
                                        } else {
                                            '_'
                                        }
                                    })
                                    .collect();
                                action = Some(StartAction::New {
                                    path: settings
                                        .projects_dir
                                        .join(safe.trim())
                                        .join("main.scene.json"),
                                    template: self.template,
                                });
                            }
                        });
                    ui.add_space(12.0);
                    ui.label(RichText::new(t("Open", "Aç")).size(16.0).strong());
                    ui.horizontal(|ui| {
                        if ui.button(t("📂 Open", "📂 Aç")).clicked()
                            && !self.open_path.trim().is_empty()
                        {
                            action = Some(StartAction::Open(PathBuf::from(self.open_path.trim())));
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.open_path)
                                .hint_text(t("Path to a .scene.json", ".scene.json dosya yolu"))
                                .desired_width(f32::INFINITY),
                        );
                    });

                    // Right: recent projects.
                    let ui = &mut cols[1];
                    ui.label(
                        RichText::new(t("Recent projects", "Son projeler"))
                            .size(16.0)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    if self.recent.is_empty() {
                        ui.label(
                            RichText::new(t(
                                "No projects yet — create one on the left.",
                                "Henüz proje yok — soldan oluştur.",
                            ))
                            .weak(),
                        );
                    }
                    egui::ScrollArea::vertical()
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            for (p, name, modified) in &self.recent {
                                let last = settings.last_scene.as_ref() == Some(p);
                                let r = egui::Frame::group(ui.style())
                                    .stroke(egui::Stroke::new(
                                        1.0_f32,
                                        if last { theme::ACCENT } else { theme::BG3 },
                                    ))
                                    .inner_margin(10)
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("🎬").size(18.0));
                                            ui.vertical(|ui| {
                                                ui.label(RichText::new(name).strong());
                                                ui.label(
                                                    RichText::new(p.display().to_string())
                                                        .size(10.5)
                                                        .weak(),
                                                );
                                            });
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(ago(*modified))
                                                            .size(11.0)
                                                            .weak(),
                                                    );
                                                },
                                            );
                                        });
                                    })
                                    .response;
                                if r.interact(egui::Sense::click())
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    action = Some(StartAction::Open(p.clone()));
                                }
                            }
                        });
                });
            });

        // Update prompt (asked on the first screen).
        let rel_opt = updater.release.lock().unwrap().clone();
        if let Some(rel) = rel_opt {
            let st = updater.state();
            let show = matches!(st, UpdateState::Available) && !updater.dismissed
                || matches!(
                    st,
                    UpdateState::Downloading(_) | UpdateState::ReadyToRestart
                );
            if show {
                egui::Window::new(t("Update available", "Güncelleme mevcut"))
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.set_width(380.0);
                        ui.horizontal(|ui| {
                            let (r, _) = ui
                                .allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                            theme::draw_logo(ui.painter(), r);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{} {}",
                                        t(
                                            "A new version of Forge is ready:",
                                            "Forge'un yeni sürümü hazır:"
                                        ),
                                        rel.title
                                    ))
                                    .strong(),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "Build {}  ›  Build {}",
                                        BUILD, rel.build
                                    ))
                                    .weak(),
                                );
                            });
                        });
                        if !rel.notes.trim().is_empty() {
                            egui::ScrollArea::vertical()
                                .max_height(120.0)
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(
                                            rel.notes.chars().take(1200).collect::<String>(),
                                        )
                                        .size(11.5),
                                    );
                                });
                        }
                        ui.add_space(6.0);
                        match st {
                            UpdateState::Downloading(p) => {
                                ui.add(
                                    egui::ProgressBar::new(p)
                                        .show_percentage()
                                        .text(t("Downloading update…", "Güncelleme indiriliyor…")),
                                );
                            }
                            UpdateState::ReadyToRestart => {
                                ui.label(t(
                                    "The update is ready. Forge will restart.",
                                    "Güncelleme hazır. Forge yeniden başlatılacak.",
                                ));
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(t("Restart now", "Şimdi yeniden başlat"))
                                                .color(Color32::WHITE),
                                        )
                                        .fill(theme::ACCENT),
                                    )
                                    .clicked()
                                {
                                    updater.restart(ctx);
                                }
                            }
                            _ => {
                                ui.label(t("Do you want to update now?", "Şimdi güncellensin mi?"));
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(t("Update", "Güncelle"))
                                                    .color(Color32::WHITE)
                                                    .strong(),
                                            )
                                            .fill(theme::ACCENT)
                                            .min_size(egui::vec2(110.0, 28.0)),
                                        )
                                        .clicked()
                                    {
                                        updater.install(ctx);
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new(t("Later", "Daha sonra"))
                                                .min_size(egui::vec2(90.0, 28.0)),
                                        )
                                        .clicked()
                                    {
                                        updater.dismissed = true;
                                    }
                                });
                            }
                        }
                    });
            }
        }
        action
    }
}

fn ago(ts: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let d = now.saturating_sub(ts);
    if d < 3600 {
        format!("{} {}", (d / 60).max(1), t("min ago", "dk önce"))
    } else if d < 86400 {
        format!("{} {}", d / 3600, t("h ago", "sa önce"))
    } else {
        format!("{} {}", d / 86400, t("days ago", "gün önce"))
    }
}
