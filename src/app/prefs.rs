//! Preferences window: categories on the left, settings on the right.

use super::{panels, Editor, LogLevel};
use crate::ai::agent::Mode;
use crate::config::Permission;
use crate::i18n::{self, t, Lang};
use crate::theme;
use egui::RichText;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    General,
    Editor,
    Rendering,
    Audio,
    Ai,
    Agents,
    Memory,
    Keys,
    Updates,
}

impl Page {
    const ALL: [Page; 9] = [
        Page::General,
        Page::Editor,
        Page::Rendering,
        Page::Audio,
        Page::Ai,
        Page::Agents,
        Page::Memory,
        Page::Keys,
        Page::Updates,
    ];
    fn label(self) -> &'static str {
        match self {
            Page::General => t("General", "Genel"),
            Page::Editor => t("Editor", "Editör"),
            Page::Rendering => t("Rendering", "Görüntü"),
            Page::Audio => t("Audio", "Ses"),
            Page::Ai => t("AI", "Yapay Zekâ"),
            Page::Agents => t("Agents & Models", "Ajanlar ve Modeller"),
            Page::Memory => t("Agent Memory", "Ajan Hafızası"),
            Page::Keys => t("API Keys", "API Anahtarları"),
            Page::Updates => t("Updates", "Güncellemeler"),
        }
    }
}

fn row(ui: &mut egui::Ui, label: &str, hint: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([220.0, 20.0], egui::Label::new(label));
        add(ui);
    });
    if !hint.is_empty() {
        ui.label(RichText::new(hint).size(10.5).weak());
    }
    ui.add_space(4.0);
}

impl Editor {
    pub fn preferences_ui(&mut self, ui: &mut egui::Ui) {
        egui::SidePanel::left("prefs_nav")
            .resizable(false)
            .exact_width(170.0)
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                for p in Page::ALL {
                    if ui
                        .selectable_label(self.prefs_page == p, p.label())
                        .clicked()
                    {
                        self.prefs_page = p;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(t("💾 Save", "💾 Kaydet"))
                                    .color(egui::Color32::WHITE),
                            )
                            .fill(theme::ACCENT),
                        )
                        .clicked()
                    {
                        self.settings.save();
                        let _ = self.secrets.save();
                        self.log(
                            LogLevel::Info,
                            t("Preferences saved", "Tercihler kaydedildi").into(),
                        );
                    }
                });
            });
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.heading(self.prefs_page.label());
            ui.separator();
            let s = &mut self.settings;
            match self.prefs_page {
                Page::General => {
                    row(ui, t("Language", "Dil"), "", |ui| {
                        for l in [Lang::En, Lang::Tr] {
                            if ui.selectable_label(s.lang == l, l.label()).clicked() {
                                s.lang = l;
                                i18n::set(l);
                            }
                        }
                    });
                    row(ui, t("Show start screen on launch", "Açılışta başlangıç ekranını göster"), t("Off: reopen the last project directly.", "Kapalı: son projeyi doğrudan açar."), |ui| {
                        ui.checkbox(&mut s.show_start_screen, "");
                    });
                    row(ui, t("Autosave (minutes, 0 = off)", "Otomatik kayıt (dakika, 0 = kapalı)"), "", |ui| {
                        ui.add(egui::DragValue::new(&mut s.autosave_minutes).range(0..=120));
                    });
                    row(ui, t("Interface scale", "Arayüz ölçeği"), t("Applied immediately.", "Hemen uygulanır."), |ui| {
                        if ui.add(egui::Slider::new(&mut s.ui_scale, 0.75..=1.75)).changed() {
                            ui.ctx().set_zoom_factor(s.ui_scale);
                        }
                    });
                    row(ui, t("Projects folder", "Projeler klasörü"), "", |ui| {
                        let mut p = s.projects_dir.display().to_string();
                        if ui.text_edit_singleline(&mut p).changed() {
                            s.projects_dir = p.into();
                        }
                    });
                }
                Page::Editor => {
                    row(ui, t("Show grid", "Izgarayı göster"), "", |ui| {
                        ui.checkbox(&mut s.show_grid, "");
                    });
                    row(ui, t("Show light/camera icons", "Işık/kamera simgelerini göster"), "", |ui| {
                        ui.checkbox(&mut s.show_icons, "");
                    });
                    row(ui, t("Stats overlay", "İstatistik katmanı"), "", |ui| {
                        ui.checkbox(&mut s.show_stats, "");
                    });
                    row(ui, t("Gizmo size", "Gizmo boyutu"), "", |ui| {
                        ui.add(egui::Slider::new(&mut s.gizmo_size, 0.5..=2.0));
                    });
                    row(ui, t("Camera fly speed", "Kamera uçuş hızı"), t("Right mouse + WASD/QE.", "Sağ tık + WASD/QE."), |ui| {
                        ui.add(egui::Slider::new(&mut s.camera_speed, 0.2..=5.0));
                    });
                    row(ui, t("Always snap", "Her zaman yapış"), t("Or hold Ctrl while dragging.", "Ya da sürüklerken Ctrl'e bas."), |ui| {
                        ui.checkbox(&mut s.snap, "");
                    });
                    row(ui, t("Snap: move / rotate° / scale", "Yapış: taşı / döndür° / ölçek"), "", |ui| {
                        ui.add(egui::DragValue::new(&mut s.snap_move).speed(0.05).range(0.01..=10.0));
                        ui.add(egui::DragValue::new(&mut s.snap_rotate).speed(0.5).range(1.0..=90.0));
                        ui.add(egui::DragValue::new(&mut s.snap_scale).speed(0.01).range(0.01..=5.0));
                    });
                }
                Page::Rendering => {
                    let env = &mut self.scene.environment;
                    row(ui, t("Real-time shadows", "Gerçek zamanlı gölgeler"), t("Per scene — also in the Lighting tab.", "Sahneye özel — Işıklandırma sekmesinde de var."), |ui| {
                        ui.checkbox(&mut env.shadows, "");
                    });
                    row(ui, t("Shadow softness", "Gölge yumuşaklığı"), "", |ui| {
                        ui.add(egui::Slider::new(&mut env.shadow_softness, 0.0..=4.0));
                    });
                    row(ui, t("Exposure", "Pozlama"), "", |ui| {
                        ui.add(egui::Slider::new(&mut env.exposure, 0.2..=3.0));
                    });
                    row(ui, t("Fog start / end", "Sis başlangıç / bitiş"), "", |ui| {
                        ui.add(egui::DragValue::new(&mut env.fog_start).range(0.0..=2000.0));
                        ui.add(egui::DragValue::new(&mut env.fog_end).range(1.0..=5000.0));
                    });
                }
                Page::Audio => {
                    row(ui, t("Master volume", "Ana ses"), "", |ui| {
                        ui.add(egui::Slider::new(&mut s.master_volume, 0.0..=2.0));
                    });
                    row(ui, t("Mute audio in the editor", "Editörde sesi kapat"), t("Built games always play sound.", "Derlenen oyunlar her zaman ses çalar."), |ui| {
                        ui.checkbox(&mut s.mute_editor_audio, "");
                    });
                }
                Page::Ai => {
                    row(ui, t("Enable AI", "Yapay zekâyı aç"), t("Off hides the Agent panel; nothing is sent to any model.", "Kapalıyken Ajan paneli gizlenir; hiçbir modele veri gitmez."), |ui| {
                        ui.checkbox(&mut s.ai_enabled, "");
                    });
                    row(ui, t("Permission mode", "İzin modu"), t("Manual: accept/reject every change, file and download. Auto: accept everything.", "Manuel: her değişikliği, dosyayı ve indirmeyi onaylarsın. Auto: hepsi otomatik kabul edilir."), |ui| {
                        ui.selectable_value(&mut s.permission, Permission::Manual, t("✋ Manual", "✋ Manuel"));
                        ui.selectable_value(&mut s.permission, Permission::Auto, "⚡ Auto");
                    });
                    row(ui, t("Default conversation mode", "Varsayılan sohbet modu"), "", |ui| {
                        for m in [Mode::Planning, Mode::Fast, Mode::Ask] {
                            ui.selectable_value(&mut s.agent_mode, m, m.label());
                        }
                    });
                    row(ui, t("Web search from this device", "Bu cihazdan web araması"), "", |ui| {
                        ui.checkbox(&mut s.agent_web, "");
                    });
                    row(ui, t("Web results per search", "Arama başına sonuç"), "", |ui| {
                        ui.add(egui::DragValue::new(&mut s.web_results).range(1..=15));
                    });
                    row(ui, t("Conversation turns sent as context", "Bağlam olarak gönderilen tur sayısı"), "", |ui| {
                        ui.add(egui::DragValue::new(&mut s.agent_context_turns).range(0..=30));
                    });
                    row(ui, t("Max builder steps", "Maks. kurucu adımı"), "", |ui| {
                        ui.add(egui::DragValue::new(&mut s.max_agent_steps).range(1..=40));
                    });
                    row(ui, t("Temperature", "Sıcaklık"), t("Lower = more precise, higher = more creative.", "Düşük = daha kesin, yüksek = daha yaratıcı."), |ui| {
                        ui.add(egui::Slider::new(&mut s.temperature, 0.0..=1.5));
                    });
                    row(ui, t("Planner / Reviewer agents", "Planlayıcı / Denetçi ajanlar"), "", |ui| {
                        ui.checkbox(&mut s.use_planner, "Planner");
                        ui.checkbox(&mut s.use_reviewer, "Reviewer");
                    });
                    self.chat.permission = s.permission;
                    self.chat.web = s.agent_web;
                }
                Page::Agents => {
                    let mut options: Vec<(String, String)> = vec![(String::new(), t("(same as chat model)", "(sohbet modeliyle aynı)").into())];
                    options.extend(crate::ai::models::CLOUD_MODELS.iter().map(|m| (m.id.to_string(), m.name.to_string())));
                    options.extend(s.custom_models.iter().map(|m| (m.clone(), m.clone())));
                    options.extend(self.library.installed_models().into_iter().map(|m| (m.clone(), crate::ai::models::display_name(&m))));
                    options.extend(self.library.ollama_models().into_iter().map(|m| (m.clone(), crate::ai::models::display_name(&m))));
                    row(ui, t("Chat model", "Sohbet modeli"), "", |ui| {
                        egui::ComboBox::from_id_salt("chat_model").selected_text(crate::ai::models::display_name(&s.selected_model)).width(260.0).show_ui(ui, |ui| {
                            for (id, n) in options.iter().skip(1) {
                                ui.selectable_value(&mut s.selected_model, id.clone(), n);
                            }
                        });
                    });
                    for (label, value) in [
                        ("🗺 Planner", &mut s.planner_model),
                        ("🔨 Builder", &mut s.builder_model),
                        ("⌨ Coder", &mut s.coder_model),
                        ("🔭 Explore", &mut s.router_model),
                        ("🔍 Reviewer", &mut s.reviewer_model),
                    ] {
                        row(ui, label, "", |ui| {
                            let shown = options.iter().find(|(id, _)| id == value).map(|(_, n)| n.clone()).unwrap_or_else(|| value.clone());
                            egui::ComboBox::from_id_salt(label).selected_text(shown).width(260.0).show_ui(ui, |ui| {
                                for (id, n) in &options {
                                    ui.selectable_value(value, id.clone(), n);
                                }
                            });
                        });
                    }
                    row(ui, t("Local runtime: context / GPU layers", "Yerel çalışma: bağlam / GPU katmanı"), "", |ui| {
                        ui.add(egui::DragValue::new(&mut s.llama_ctx).range(1024..=262144).speed(256));
                        ui.add(egui::DragValue::new(&mut s.llama_gpu_layers).range(0..=999));
                    });
                    row(ui, t("Ollama URL", "Ollama adresi"), "", |ui| {
                        ui.text_edit_singleline(&mut s.ollama_url);
                    });
                }
                Page::Memory => {
                    panels::memory_settings(ui, s, &self.memory_status);
                }
                Page::Keys => {
                    row(ui, "OpenRouter", t("Stored only on this PC (.env in Forge's data folder).", "Yalnızca bu bilgisayarda saklanır (veri klasöründeki .env)."), |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.secrets.openrouter_key).password(true).desired_width(320.0));
                    });
                    row(ui, "Hugging Face", t("Optional — gated models and faster downloads.", "İsteğe bağlı — kısıtlı modeller ve hızlı indirme."), |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.secrets.hf_token).password(true).desired_width(320.0));
                    });
                    ui.label(RichText::new(crate::config::env_file().display().to_string()).size(10.5).weak());
                }
                Page::Updates => {
                    row(ui, t("Check for updates automatically", "Güncellemeleri otomatik denetle"), t("On start-up and every 30 minutes.", "Açılışta ve her 30 dakikada bir."), |ui| {
                        ui.checkbox(&mut s.auto_update_check, "");
                    });
                    ui.label(format!("Forge v{} · build {} · {}", env!("CARGO_PKG_VERSION"), crate::updater::BUILD, crate::updater::COMMIT));
                    ui.hyperlink_to("github.com/burkiuze/Forge/releases", "https://github.com/burkiuze/Forge/releases");
                }
            }
        });
    }
}
