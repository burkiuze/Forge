//! Editor panels: hierarchy, inspector, lighting, project settings, console, script.

use super::{Editor, LogLevel, Tab};
use crate::ai::memory::{Memory, MemoryBackend};
use crate::config::Settings;
use crate::i18n::{self, t, Lang};
use crate::scene::{CameraComp, EntityId, Light, LightKind, MeshKind, Pattern, Rigidbody, LAYERS};
use crate::templates::SCRIPT_TEMPLATES;
use crate::theme::{self, ACCENT};
use egui::{Color32, RichText};
use std::sync::{Arc, Mutex};

pub fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .default_open(true)
        .show(ui, body);
}

fn section_removable(
    ui: &mut egui::Ui,
    title: &str,
    keep: &mut bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    let id = ui.make_persistent_id(title);
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            ui.label(RichText::new(title).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("✖")
                    .on_hover_text(t("Remove component", "Bileşeni kaldır"))
                    .clicked()
                {
                    *keep = false;
                }
            });
        })
        .body(body);
}

fn vec3_row(ui: &mut egui::Ui, label: &str, v: &mut glam::Vec3, speed: f64) -> bool {
    let mut changed = false;
    ui.label(label);
    for (c, axis) in [
        (Color32::from_rgb(230, 80, 70), &mut v.x),
        (Color32::from_rgb(110, 200, 90), &mut v.y),
        (Color32::from_rgb(80, 140, 240), &mut v.z),
    ] {
        let r = ui.add(
            egui::DragValue::new(axis)
                .speed(speed)
                .max_decimals(3)
                .custom_formatter(|n, _| format!("{n:.2}")),
        );
        ui.painter().line_segment(
            [r.rect.left_top(), r.rect.left_bottom()],
            egui::Stroke::new(2.0_f32, c),
        );
        changed |= r.changed();
    }
    ui.end_row();
    changed
}

impl Editor {
    // ------------------------------------------------------------ hierarchy

    pub fn hierarchy_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.menu_button(RichText::new("+").size(15.0), |ui| self.create_menu(ui));
            ui.add(
                egui::TextEdit::singleline(&mut self.hierarchy_filter)
                    .hint_text(t("🔍 Search", "🔍 Ara"))
                    .desired_width(f32::INFINITY),
            );
        });
        ui.label(
            RichText::new(format!(
                "🎬 {}{}",
                self.scene.name,
                if self.dirty { " *" } else { "" }
            ))
            .strong(),
        );
        let mut reparent: Option<(EntityId, Option<EntityId>)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let filter = self.hierarchy_filter.to_lowercase();
                if filter.is_empty() {
                    for id in self.scene.children(None) {
                        self.tree_node(ui, id, 0, &mut reparent);
                    }
                } else {
                    let ids: Vec<EntityId> = self
                        .scene
                        .entities
                        .iter()
                        .filter(|e| e.name.to_lowercase().contains(&filter))
                        .map(|e| e.id)
                        .collect();
                    for id in ids {
                        self.entity_row(ui, id, 0, &mut reparent);
                    }
                }
                // Drop zone: un-parent.
                let rest = ui.available_rect_before_wrap();
                let rest = egui::Rect::from_min_size(
                    rest.min,
                    egui::vec2(rest.width(), rest.height().max(30.0)),
                );
                let resp = ui.allocate_rect(rest, egui::Sense::click());
                if let Some(p) = resp.dnd_hover_payload::<EntityId>() {
                    ui.painter().rect_stroke(
                        rest.shrink(2.0),
                        4.0,
                        egui::Stroke::new(1.0_f32, ACCENT),
                        egui::StrokeKind::Inside,
                    );
                    let _ = p;
                }
                if let Some(p) = resp.dnd_release_payload::<EntityId>() {
                    reparent = Some((*p, None));
                }
                if resp.clicked() {
                    self.selected = None;
                }
            });
        if let Some((child, parent)) = reparent {
            if self.scene.get(child).map(|e| e.parent) != Some(parent) {
                self.push_undo();
                self.scene.set_parent(child, parent);
            }
        }
    }

    fn tree_node(
        &mut self,
        ui: &mut egui::Ui,
        id: EntityId,
        depth: usize,
        reparent: &mut Option<(EntityId, Option<EntityId>)>,
    ) {
        self.entity_row(ui, id, depth, reparent);
        if depth < 16 {
            for c in self.scene.children(Some(id)) {
                self.tree_node(ui, c, depth + 1, reparent);
            }
        }
    }

    fn entity_row(
        &mut self,
        ui: &mut egui::Ui,
        id: EntityId,
        depth: usize,
        reparent: &mut Option<(EntityId, Option<EntityId>)>,
    ) {
        let Some(e) = self.scene.get(id) else { return };
        let icon = if e.camera.is_some() {
            "🎥"
        } else if let Some(l) = &e.light {
            if l.kind == LightKind::Point {
                "💡"
            } else {
                "☀"
            }
        } else if !e.script.is_empty() {
            "📜"
        } else if e.mesh == MeshKind::None {
            "○"
        } else {
            "◼"
        };
        let mut text = RichText::new(format!("{icon} {}", e.name));
        if !e.active {
            text = text.weak();
        }
        let sel = self.selected == Some(id);
        let row = ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 14.0);
            let r = ui
                .selectable_label(sel, text)
                .interact(egui::Sense::click_and_drag());
            if r.dragged() {
                r.dnd_set_drag_payload(id);
            }
            r
        });
        let r = row.inner;
        if let Some(p) = r.dnd_hover_payload::<EntityId>() {
            if *p != id {
                ui.painter().rect_stroke(
                    r.rect.expand(1.0),
                    3.0,
                    egui::Stroke::new(1.0_f32, ACCENT),
                    egui::StrokeKind::Outside,
                );
            }
        }
        if let Some(p) = r.dnd_release_payload::<EntityId>() {
            if *p != id {
                *reparent = Some((*p, Some(id)));
            }
        }
        if r.clicked() {
            self.selected = Some(id);
        }
        if r.double_clicked() {
            self.focus(id);
        }
        r.context_menu(|ui| {
            self.selected = Some(id);
            if ui.button(t("Focus", "Odaklan")).clicked() {
                self.focus(id);
                ui.close_menu();
            }
            if ui.button(t("Duplicate", "Çoğalt")).clicked() {
                self.duplicate_selected();
                ui.close_menu();
            }
            if ui.button(t("Unparent", "Ebeveynden ayır")).clicked() {
                *reparent = Some((id, None));
                ui.close_menu();
            }
            if ui.button(t("Create Prefab", "Prefab oluştur")).clicked() {
                let name = self
                    .scene
                    .get(id)
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                self.dialogs.prefab_name = Some((id, name));
                ui.close_menu();
            }
            if ui.button(t("Delete", "Sil")).clicked() {
                self.delete_selected();
                ui.close_menu();
            }
            ui.separator();
            if ui
                .button(t("✨ Ask the agent about this", "✨ Bunu ajana sor"))
                .clicked()
            {
                let name = self
                    .scene
                    .get(id)
                    .map(|e| e.name.replace(' ', "_"))
                    .unwrap_or_default();
                self.chat.input = format!("@{name} ");
                self.chat.focus();
                ui.close_menu();
            }
        });
    }

    pub fn focus(&mut self, id: EntityId) {
        let m = self.scene.world_matrix(id);
        let (s, _, p) = m.to_scale_rotation_translation();
        self.cam.target = p;
        self.cam.distance = (s.max_element() * 3.0).clamp(3.0, 80.0);
    }

    // ------------------------------------------------------------ inspector

    pub fn inspector_ui(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(t(
                        "Select an object to inspect",
                        "İncelemek için bir nesne seç",
                    ))
                    .weak(),
                )
            });
            return;
        };
        let Some(before) = self.scene.get(id).cloned() else {
            self.selected = None;
            return;
        };
        let mut e = before.clone();
        let mut changed = false;
        let mut remove = false;
        let entity_names: Vec<String> = self
            .scene
            .entities
            .iter()
            .filter(|x| x.id != id)
            .map(|x| x.name.clone())
            .collect();
        let textures = self.project_textures();
        let audio_clips = list_rel(&self.project_root(), "Audio", &crate::audio::AUDIO_EXTS);
        let anim_clips = self.animation_clips();
        let mut preview_sound: Option<String> = None;

        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut e.active, "").changed();
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut e.name)
                        .desired_width(ui.available_width() - 70.0),
                )
                .changed();
            changed |= ui.checkbox(&mut e.is_static, "Static").changed();
        });
        ui.horizontal(|ui| {
            ui.label("Tag");
            changed |= ui
                .add(egui::TextEdit::singleline(&mut e.tag).desired_width(80.0))
                .changed();
            ui.label(t("Layer", "Katman"));
            egui::ComboBox::from_id_salt("layer")
                .selected_text(e.layer.clone())
                .width(100.0)
                .show_ui(ui, |ui| {
                    for l in LAYERS {
                        changed |= ui
                            .selectable_value(&mut e.layer, l.to_string(), l)
                            .changed();
                    }
                });
        });
        ui.separator();
        section(ui, "Transform", |ui| {
            egui::Grid::new("tf")
                .num_columns(4)
                .spacing([4.0, 4.0])
                .show(ui, |ui| {
                    changed |=
                        vec3_row(ui, t("Position", "Konum"), &mut e.transform.position, 0.05);
                    changed |= vec3_row(
                        ui,
                        t("Rotation", "Döndürme"),
                        &mut e.transform.rotation,
                        0.5,
                    );
                    changed |= vec3_row(ui, t("Scale", "Ölçek"), &mut e.transform.scale, 0.02);
                });
        });
        if e.mesh != MeshKind::None {
            section(ui, "Mesh Renderer", |ui| {
                egui::Grid::new("mr")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Mesh");
                        egui::ComboBox::from_id_salt("mesh")
                            .selected_text(e.mesh.label())
                            .show_ui(ui, |ui| {
                                for m in MeshKind::ALL {
                                    changed |=
                                        ui.selectable_value(&mut e.mesh, m, m.label()).changed();
                                }
                            });
                        ui.end_row();
                        ui.label(t("Albedo", "Renk"));
                        changed |= ui.color_edit_button_rgb(&mut e.color).changed();
                        ui.end_row();
                        let m = &mut e.material;
                        ui.label(t("Texture", "Doku"));
                        egui::ComboBox::from_id_salt("tex")
                            .selected_text(if m.texture.is_empty() {
                                t("None", "Yok").to_string()
                            } else {
                                m.texture.clone()
                            })
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(
                                        &mut m.texture,
                                        String::new(),
                                        t("None", "Yok"),
                                    )
                                    .changed();
                                for tx in &textures {
                                    changed |= ui
                                        .selectable_value(&mut m.texture, tx.clone(), tx)
                                        .changed();
                                }
                            });
                        ui.end_row();
                        ui.label(t("Tiling", "Döşeme"));
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut m.tiling)
                                    .speed(0.05)
                                    .range(0.05..=100.0),
                            )
                            .changed();
                        ui.end_row();
                        ui.label(t("Pattern", "Desen"));
                        egui::ComboBox::from_id_salt("pat")
                            .selected_text(format!("{:?}", m.pattern))
                            .show_ui(ui, |ui| {
                                for p in [Pattern::None, Pattern::Checker, Pattern::Grid] {
                                    changed |= ui
                                        .selectable_value(&mut m.pattern, p, format!("{p:?}"))
                                        .changed();
                                }
                            });
                        ui.end_row();
                        ui.label("Metallic");
                        changed |= ui
                            .add(egui::Slider::new(&mut m.metallic, 0.0..=1.0))
                            .changed();
                        ui.end_row();
                        ui.label("Roughness");
                        changed |= ui
                            .add(egui::Slider::new(&mut m.roughness, 0.0..=1.0))
                            .changed();
                        ui.end_row();
                        ui.label(t("Emission", "Işıma"));
                        ui.horizontal(|ui| {
                            changed |= ui.color_edit_button_rgb(&mut m.emission).changed();
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut m.emission_strength)
                                        .speed(0.02)
                                        .range(0.0..=10.0),
                                )
                                .changed();
                        });
                        ui.end_row();
                        ui.label("Unlit");
                        changed |= ui.checkbox(&mut m.unlit, "").changed();
                        ui.end_row();
                    });
            });
            section(ui, t("Collider", "Çarpıştırıcı"), |ui| {
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut e.collider, "Box Collider").changed();
                    changed |= ui.checkbox(&mut e.trigger, "Is Trigger").changed();
                });
            });
        }
        if let Some(rb) = &mut e.rigidbody {
            let mut keep = true;
            section_removable(ui, "Rigidbody", &mut keep, |ui| {
                egui::Grid::new("rb").num_columns(2).show(ui, |ui| {
                    ui.label(t("Mass", "Kütle"));
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut rb.mass)
                                .speed(0.05)
                                .range(0.01..=1000.0),
                        )
                        .changed();
                    ui.end_row();
                    ui.label(t("Bounciness", "Sekme"));
                    changed |= ui
                        .add(egui::Slider::new(&mut rb.bounciness, 0.0..=1.0))
                        .changed();
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut rb.use_gravity, "Use Gravity").changed();
                    changed |= ui.checkbox(&mut rb.is_static, "Kinematic/Static").changed();
                });
            });
            if !keep {
                e.rigidbody = None;
                changed = true;
            }
        }
        if let Some(l) = &mut e.light {
            let mut keep = true;
            section_removable(ui, t("Light", "Işık"), &mut keep, |ui| {
                egui::Grid::new("light").num_columns(2).show(ui, |ui| {
                    ui.label(t("Type", "Tür"));
                    egui::ComboBox::from_id_salt("lk")
                        .selected_text(format!("{:?}", l.kind))
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(
                                    &mut l.kind,
                                    LightKind::Directional,
                                    "Directional",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(&mut l.kind, LightKind::Point, "Point")
                                .changed();
                        });
                    ui.end_row();
                    ui.label(t("Color", "Renk"));
                    changed |= ui.color_edit_button_rgb(&mut l.color).changed();
                    ui.end_row();
                    ui.label(t("Intensity", "Yoğunluk"));
                    changed |= ui
                        .add(egui::Slider::new(&mut l.intensity, 0.0..=8.0))
                        .changed();
                    ui.end_row();
                    if l.kind == LightKind::Point {
                        ui.label(t("Range", "Menzil"));
                        changed |= ui
                            .add(egui::Slider::new(&mut l.range, 0.5..=60.0))
                            .changed();
                        ui.end_row();
                    }
                });
            });
            if !keep {
                e.light = None;
                changed = true;
            }
        }
        if let Some(c) = &mut e.camera {
            let mut keep = true;
            section_removable(ui, t("Camera", "Kamera"), &mut keep, |ui| {
                ui.horizontal(|ui| {
                    ui.label("FOV");
                    changed |= ui
                        .add(egui::Slider::new(&mut c.fov, 20.0..=120.0))
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label(t("Follow", "Takip"));
                    egui::ComboBox::from_id_salt("follow")
                        .selected_text(if c.follow.is_empty() {
                            "(none)"
                        } else {
                            c.follow.as_str()
                        })
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(&mut c.follow, String::new(), "(none)")
                                .changed();
                            for n in &entity_names {
                                changed |=
                                    ui.selectable_value(&mut c.follow, n.clone(), n).changed();
                            }
                        });
                });
            });
            if !keep {
                e.camera = None;
                changed = true;
            }
        }
        if let Some(a) = &mut e.audio {
            let mut keep = true;
            section_removable(ui, "Audio Source", &mut keep, |ui| {
                egui::Grid::new("audio").num_columns(2).show(ui, |ui| {
                    ui.label(t("Clip", "Klip"));
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("aclip")
                            .selected_text(if a.clip.is_empty() {
                                t("None", "Yok").to_string()
                            } else {
                                a.clip.clone()
                            })
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                for c in &audio_clips {
                                    changed |=
                                        ui.selectable_value(&mut a.clip, c.clone(), c).changed();
                                }
                            });
                        if ui
                            .small_button("▶")
                            .on_hover_text(t("Preview", "Dinle"))
                            .clicked()
                            && !a.clip.is_empty()
                        {
                            preview_sound = Some(a.clip.clone());
                        }
                    });
                    ui.end_row();
                    ui.label(t("Volume", "Ses seviyesi"));
                    changed |= ui
                        .add(egui::Slider::new(&mut a.volume, 0.0..=2.0))
                        .changed();
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut a.looping, "Loop").changed();
                    changed |= ui.checkbox(&mut a.play_on_start, "Play On Start").changed();
                });
                if audio_clips.is_empty() {
                    ui.label(
                        RichText::new(t(
                            "No clips yet — Asset Store › Audio or import from device.",
                            "Henüz klip yok — Varlık Mağazası › Ses ya da cihazdan içe aktar.",
                        ))
                        .size(10.5)
                        .weak(),
                    );
                }
            });
            if !keep {
                e.audio = None;
                changed = true;
            }
        }
        if let Some(a) = &mut e.animator {
            let mut keep = true;
            section_removable(ui, "Animator", &mut keep, |ui| {
                egui::Grid::new("animator").num_columns(2).show(ui, |ui| {
                    ui.label(t("Clip", "Klip"));
                    egui::ComboBox::from_id_salt("anclip")
                        .selected_text(a.clip.clone())
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for c in &anim_clips {
                                changed |= ui.selectable_value(&mut a.clip, c.clone(), c).changed();
                            }
                        });
                    ui.end_row();
                    ui.label(t("Speed", "Hız"));
                    changed |= ui.add(egui::Slider::new(&mut a.speed, 0.1..=4.0)).changed();
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut a.play_on_start, "Play On Start").changed();
                    if ui
                        .small_button(t("🎞 Edit clip", "🎞 Klibi düzenle"))
                        .clicked()
                    {
                        self.open_tab = Some(Tab::Animation);
                    }
                });
            });
            if !keep {
                e.animator = None;
                changed = true;
            }
        }
        if !e.script.is_empty() {
            let mut keep = true;
            section_removable(ui, t("Script (Rhai)", "Betik (Rhai)"), &mut keep, |ui| {
                ui.horizontal(|ui| {
                    ui.small(format!(
                        "{} {}",
                        e.script.lines().count(),
                        t("lines", "satır")
                    ));
                    if ui.button(t("✏ Edit", "✏ Düzenle")).clicked() {
                        self.open_tab = Some(Tab::Script);
                    }
                });
            });
            if !keep {
                e.script.clear();
                changed = true;
            }
        }
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            ui.menu_button(
                RichText::new(t("Add Component", "Bileşen Ekle")).strong(),
                |ui| {
                    ui.set_min_width(220.0);
                    let sid = ui.make_persistent_id("ac_search");
                    let mut q: String = ui.data_mut(|d| d.get_temp(sid).unwrap_or_default());
                    ui.add(egui::TextEdit::singleline(&mut q).hint_text(t("Search…", "Ara…")));
                    let ql = q.to_lowercase();
                    let show = |name: &str| ql.is_empty() || name.to_lowercase().contains(&ql);
                    ui.separator();
                    let add = |ui: &mut egui::Ui, name: &str, enabled: bool| -> bool {
                        show(name) && ui.add_enabled(enabled, egui::Button::new(name)).clicked()
                    };
                    ui.label(RichText::new(t("Physics", "Fizik")).small().weak());
                    if add(ui, "Rigidbody", e.rigidbody.is_none()) {
                        e.rigidbody = Some(Rigidbody::default());
                        changed = true;
                        ui.close_menu();
                    }
                    if add(ui, "Box Collider", !e.collider) {
                        e.collider = true;
                        changed = true;
                        ui.close_menu();
                    }
                    if add(ui, "Trigger", !e.trigger) {
                        e.trigger = true;
                        changed = true;
                        ui.close_menu();
                    }
                    ui.label(RichText::new(t("Rendering", "Görüntüleme")).small().weak());
                    for m in MeshKind::ALL.iter().skip(1) {
                        if add(ui, &format!("Mesh: {}", m.label()), e.mesh != *m) {
                            e.mesh = *m;
                            changed = true;
                            ui.close_menu();
                        }
                    }
                    if add(ui, "Directional Light", e.light.is_none()) {
                        e.light = Some(Light::default());
                        changed = true;
                        ui.close_menu();
                    }
                    if add(ui, "Point Light", e.light.is_none()) {
                        e.light = Some(Light::point());
                        changed = true;
                        ui.close_menu();
                    }
                    if add(ui, "Camera", e.camera.is_none()) {
                        e.camera = Some(CameraComp::default());
                        changed = true;
                        ui.close_menu();
                    }
                    ui.label(RichText::new(t("Audio", "Ses")).small().weak());
                    if add(ui, "Audio Source", e.audio.is_none()) {
                        e.audio = Some(crate::scene::AudioSource {
                            clip: audio_clips.first().cloned().unwrap_or_default(),
                            ..Default::default()
                        });
                        changed = true;
                        ui.close_menu();
                    }
                    ui.label(RichText::new(t("Animation", "Animasyon")).small().weak());
                    if add(ui, "Animator", e.animator.is_none()) {
                        e.animator = Some(crate::scene::Animator::default());
                        changed = true;
                        ui.close_menu();
                    }
                    ui.label(RichText::new(t("Scripts", "Betikler")).small().weak());
                    for (name, code) in SCRIPT_TEMPLATES {
                        if add(ui, name, true) {
                            e.script = code.to_string();
                            changed = true;
                            ui.close_menu();
                        }
                    }
                    for f in self.project_scripts() {
                        if add(ui, &f, true) {
                            if let Ok(code) = std::fs::read_to_string(self.project_root().join(&f))
                            {
                                e.script = code;
                                changed = true;
                            }
                            ui.close_menu();
                        }
                    }
                    ui.data_mut(|d| d.insert_temp(sid, q));
                },
            );
            ui.add_space(4.0);
            if ui
                .button(RichText::new(t("🗑 Delete", "🗑 Sil")).color(Color32::LIGHT_RED))
                .clicked()
            {
                remove = true;
            }
        });

        if let Some(c) = preview_sound {
            let p = self.project_root().join(c);
            if let Err(err) = self.audio.play(&p, 1.0, None) {
                self.log(LogLevel::Warn, err);
            }
        }
        if remove {
            self.delete_selected();
            return;
        }
        if changed {
            if !self.editing {
                // One undo step per interaction: snapshot the pre-edit state.
                if let Some(slot) = self.scene.get_mut(id) {
                    *slot = before;
                }
                self.push_undo();
                self.editing = true;
            }
            if self
                .scene
                .entities
                .iter()
                .any(|x| x.id != id && x.name == e.name)
            {
                e.name = self.scene.unique_name(&e.name);
            }
            if let Some(slot) = self.scene.get_mut(id) {
                *slot = e;
            }
            self.dirty = true;
        }
        if !ui.ctx().input(|i| i.pointer.any_down()) && !ui.ctx().wants_keyboard_input() {
            self.editing = false;
        }
    }

    pub fn project_textures(&self) -> Vec<String> {
        list_rel(&self.project_root(), "Textures", &["png", "jpg", "jpeg"])
    }

    pub fn project_scripts(&self) -> Vec<String> {
        list_rel(&self.project_root(), "Scripts", &["rhai"])
    }

    // ------------------------------------------------------------ lighting

    pub fn lighting_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.label(RichText::new(t("Presets", "Hazır ayarlar")).strong());
        ui.horizontal_wrapped(|ui| {
            for (name, top, bottom, sun_col, intensity, ambient, rot) in [
                (
                    t("Day", "Gündüz"),
                    [0.20, 0.36, 0.62],
                    [0.70, 0.74, 0.78],
                    [1.0, 0.96, 0.9],
                    1.1,
                    0.38,
                    [-55.0, 30.0],
                ),
                (
                    t("Sunset", "Gün batımı"),
                    [0.18, 0.18, 0.36],
                    [0.95, 0.55, 0.35],
                    [1.0, 0.62, 0.35],
                    1.2,
                    0.30,
                    [-12.0, 60.0],
                ),
                (
                    t("Night", "Gece"),
                    [0.02, 0.03, 0.08],
                    [0.08, 0.09, 0.16],
                    [0.55, 0.65, 1.0],
                    0.25,
                    0.18,
                    [-40.0, -20.0],
                ),
                (
                    t("Overcast", "Kapalı"),
                    [0.46, 0.48, 0.52],
                    [0.62, 0.63, 0.65],
                    [0.85, 0.86, 0.9],
                    0.55,
                    0.62,
                    [-70.0, 10.0],
                ),
            ] {
                if ui.button(name).clicked() {
                    self.push_undo();
                    let env = &mut self.scene.environment;
                    env.sky_top = top;
                    env.sky_bottom = bottom;
                    env.ambient = ambient;
                    if let Some(ent) = self.scene.entities.iter_mut().find(|e| {
                        e.light
                            .as_ref()
                            .map_or(false, |l| l.kind == LightKind::Directional)
                    }) {
                        let l = ent.light.as_mut().unwrap();
                        l.color = sun_col;
                        l.intensity = intensity;
                        ent.transform.rotation = glam::Vec3::new(rot[0], rot[1], 0.0);
                    }
                    changed = true;
                }
            }
        });
        ui.separator();
        let env = &mut self.scene.environment;
        section(ui, t("Environment", "Ortam"), |ui| {
            egui::Grid::new("env")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label(t("Sky (zenith)", "Gökyüzü (tepe)"));
                    changed |= ui.color_edit_button_rgb(&mut env.sky_top).changed();
                    ui.end_row();
                    ui.label(t("Sky (horizon)", "Gökyüzü (ufuk)"));
                    changed |= ui.color_edit_button_rgb(&mut env.sky_bottom).changed();
                    ui.end_row();
                    ui.label(t("Ambient", "Ortam ışığı"));
                    changed |= ui
                        .add(egui::Slider::new(&mut env.ambient, 0.0..=1.5))
                        .changed();
                    ui.end_row();
                    ui.label(t("Exposure", "Pozlama"));
                    changed |= ui
                        .add(egui::Slider::new(&mut env.exposure, 0.2..=3.0))
                        .changed();
                    ui.end_row();
                    ui.label(t("Fog start", "Sis başlangıcı"));
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut env.fog_start)
                                .speed(1.0)
                                .range(0.0..=2000.0),
                        )
                        .changed();
                    ui.end_row();
                    ui.label(t("Fog end", "Sis sonu"));
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut env.fog_end)
                                .speed(1.0)
                                .range(1.0..=5000.0),
                        )
                        .changed();
                    ui.end_row();
                });
        });
        section(ui, t("Shadows", "Gölgeler"), |ui| {
            changed |= ui
                .checkbox(
                    &mut env.shadows,
                    t("Real-time shadows (sun)", "Gerçek zamanlı gölgeler (güneş)"),
                )
                .changed();
            ui.horizontal(|ui| {
                ui.label(t("Softness", "Yumuşaklık"));
                changed |= ui
                    .add(egui::Slider::new(&mut env.shadow_softness, 0.0..=4.0))
                    .changed();
            });
        });
        let sun = self.scene.entities.iter_mut().find(|e| {
            e.light
                .as_ref()
                .map_or(false, |l| l.kind == LightKind::Directional)
        });
        section(ui, t("Sun", "Güneş"), |ui| match sun {
            Some(s) => {
                let l = s.light.as_mut().unwrap();
                egui::Grid::new("sun").num_columns(2).show(ui, |ui| {
                    ui.label(t("Color", "Renk"));
                    changed |= ui.color_edit_button_rgb(&mut l.color).changed();
                    ui.end_row();
                    ui.label(t("Intensity", "Yoğunluk"));
                    changed |= ui
                        .add(egui::Slider::new(&mut l.intensity, 0.0..=4.0))
                        .changed();
                    ui.end_row();
                    ui.label(t("Elevation", "Yükseklik"));
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut s.transform.rotation.x, -90.0..=0.0).suffix("°"),
                        )
                        .changed();
                    ui.end_row();
                    ui.label(t("Azimuth", "Yön"));
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut s.transform.rotation.y, -180.0..=180.0)
                                .suffix("°"),
                        )
                        .changed();
                    ui.end_row();
                });
            }
            None => {
                ui.label(
                    RichText::new(t(
                        "No directional light in the scene.",
                        "Sahnede yönlü ışık yok.",
                    ))
                    .weak(),
                );
            }
        });
        let points = self
            .scene
            .entities
            .iter()
            .filter(|e| {
                e.light
                    .as_ref()
                    .map_or(false, |l| l.kind == LightKind::Point)
            })
            .count();
        ui.label(
            RichText::new(format!(
                "💡 {points}/8 {}",
                t("point lights", "noktasal ışık")
            ))
            .small()
            .weak(),
        );
        if ui
            .button(t("+ Add point light", "+ Noktasal ışık ekle"))
            .clicked()
        {
            let id = self.create("Point Light", MeshKind::None);
            let e = self.scene.get_mut(id).unwrap();
            e.light = Some(Light::point());
            e.transform.position.y = 2.5;
        }
        if changed {
            self.dirty = true;
        }
    }

    // ------------------------------------------------------------ project settings

    pub fn project_settings_ui(&mut self, ui: &mut egui::Ui) {
        section(ui, t("General", "Genel"), |ui| {
            egui::Grid::new("gen")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label(t("Language", "Dil"));
                    ui.horizontal(|ui| {
                        for l in [Lang::En, Lang::Tr] {
                            if ui
                                .selectable_label(self.settings.lang == l, l.label())
                                .clicked()
                            {
                                self.settings.lang = l;
                                i18n::set(l);
                                self.settings.save();
                            }
                        }
                    });
                    ui.end_row();
                    ui.label(t("Scene name", "Sahne adı"));
                    if ui.text_edit_singleline(&mut self.scene.name).changed() {
                        self.dirty = true;
                    }
                    ui.end_row();
                    ui.label(t("Project folder", "Proje klasörü"));
                    ui.label(RichText::new(self.project_root().display().to_string()).small());
                    ui.end_row();
                });
        });
        section(ui, t("Physics", "Fizik"), |ui| {
            ui.horizontal(|ui| {
                ui.label(t("Gravity (m/s²)", "Yerçekimi (m/s²)"));
                if ui
                    .add(egui::DragValue::new(&mut self.scene.environment.gravity).speed(0.1))
                    .changed()
                {
                    self.dirty = true;
                }
            });
        });
        section(ui, t("Editor", "Editör"), |ui| {
            let s = &mut self.settings;
            ui.checkbox(
                &mut s.snap,
                t(
                    "Snap while dragging gizmos (or hold Ctrl)",
                    "Gizmo sürüklerken yapış (veya Ctrl)",
                ),
            );
            egui::Grid::new("snap").num_columns(2).show(ui, |ui| {
                ui.label(t("Move snap", "Taşıma adımı"));
                ui.add(
                    egui::DragValue::new(&mut s.snap_move)
                        .speed(0.05)
                        .range(0.01..=10.0),
                );
                ui.end_row();
                ui.label(t("Rotate snap (°)", "Döndürme adımı (°)"));
                ui.add(
                    egui::DragValue::new(&mut s.snap_rotate)
                        .speed(0.5)
                        .range(1.0..=90.0),
                );
                ui.end_row();
                ui.label(t("Scale snap", "Ölçek adımı"));
                ui.add(
                    egui::DragValue::new(&mut s.snap_scale)
                        .speed(0.01)
                        .range(0.01..=5.0),
                );
                ui.end_row();
            });
            ui.checkbox(&mut s.show_grid, t("Show grid", "Izgarayı göster"));
            ui.checkbox(
                &mut s.show_stats,
                t("Show stats overlay", "İstatistikleri göster"),
            );
        });
        section(ui, t("AI & Agent Memory", "AI ve Ajan Hafızası"), |ui| {
            memory_settings(ui, &mut self.settings, &self.memory_status);
            if ui
                .button(t("Open AI settings…", "AI ayarlarını aç…"))
                .clicked()
            {
                self.dialogs.settings = true;
            }
        });
        section(ui, t("Build", "Derleme"), |ui| {
            if ui.button(t("🔨 Build Game…", "🔨 Oyunu Derle…")).clicked() {
                self.dialogs.build = Some(
                    self.settings
                        .projects_dir
                        .join("Builds")
                        .join(super::sanitize(&self.scene.name))
                        .display()
                        .to_string(),
                );
            }
        });
        if ui.button(t("Save settings", "Ayarları kaydet")).clicked() {
            self.settings.save();
        }
    }

    // ------------------------------------------------------------ console

    pub fn console_ui(&mut self, ui: &mut egui::Ui) {
        let counts = [
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
            LogLevel::Ai,
        ]
        .map(|l| self.console.iter().filter(|(x, _)| *x == l).count());
        ui.horizontal(|ui| {
            if ui.button(t("Clear", "Temizle")).clicked() {
                self.console.clear();
            }
            ui.separator();
            for (i, (icon, c)) in [
                ("ℹ", theme::BONE),
                ("⚠", theme::GOLD),
                ("⛔", Color32::LIGHT_RED),
                ("✨", ACCENT),
            ]
            .iter()
            .enumerate()
            {
                ui.toggle_value(
                    &mut self.console_filter[i],
                    RichText::new(format!("{icon} {}", counts[i])).color(*c),
                );
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.console_search)
                    .hint_text(t("🔍 Filter", "🔍 Filtrele"))
                    .desired_width(160.0),
            );
        });
        ui.separator();
        let q = self.console_search.to_lowercase();
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for (lvl, msg) in &self.console {
                    let idx = match lvl {
                        LogLevel::Info => 0,
                        LogLevel::Warn => 1,
                        LogLevel::Error => 2,
                        LogLevel::Ai => 3,
                    };
                    if !self.console_filter[idx]
                        || (!q.is_empty() && !msg.to_lowercase().contains(&q))
                    {
                        continue;
                    }
                    let (icon, c) = match lvl {
                        LogLevel::Info => ("ℹ", theme::BONE),
                        LogLevel::Warn => ("⚠", theme::GOLD),
                        LogLevel::Error => ("⛔", Color32::LIGHT_RED),
                        LogLevel::Ai => ("✨", ACCENT),
                    };
                    ui.label(
                        RichText::new(format!("{icon} {msg}"))
                            .monospace()
                            .size(11.5)
                            .color(c),
                    );
                }
            });
    }

    // ------------------------------------------------------------ script

    pub fn script_ui(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            ui.label(
                RichText::new(t(
                    "Select an entity to edit its script.",
                    "Betiğini düzenlemek için bir nesne seç.",
                ))
                .weak(),
            );
            return;
        };
        let name = self
            .scene
            .get(id)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        let mut code = self
            .scene
            .get(id)
            .map(|e| e.script.clone())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("📜 {name}")).strong());
            if ui.button(t("✔ Check", "✔ Denetle")).clicked() {
                match rhai::Engine::new().compile(&code) {
                    Ok(_) => self.log(
                        LogLevel::Info,
                        format!("{name}: {}", t("script compiles", "betik derleniyor")),
                    ),
                    Err(e) => self.log(LogLevel::Error, format!("{name}: {e}")),
                }
            }
            if ui
                .button(t("💾 Save to Scripts/", "💾 Scripts/ içine kaydet"))
                .clicked()
            {
                let dir = self.project_root().join("Scripts");
                let _ = std::fs::create_dir_all(&dir);
                let p = dir.join(format!("{}.rhai", super::sanitize(&name).replace(' ', "_")));
                match std::fs::write(&p, &code) {
                    Ok(()) => self.log(
                        LogLevel::Info,
                        format!("{} {}", t("Saved", "Kaydedildi:"), p.display()),
                    ),
                    Err(e) => self.log(LogLevel::Error, e.to_string()),
                }
            }
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut self.dialogs.script_goal)
                    .hint_text(t(
                        "✨ What should this object do?",
                        "✨ Bu nesne ne yapsın?",
                    ))
                    .desired_width(240.0),
            );
            if ui
                .add_enabled(
                    !self.chat.busy() && !self.dialogs.script_goal.trim().is_empty(),
                    egui::Button::new(t("Write with AI", "AI ile yaz")),
                )
                .clicked()
            {
                self.chat.input = format!(
                    "{} '{name}'. {}: {}",
                    t(
                        "Write a Rhai script for the entity",
                        "Şu nesne için bir Rhai betiği yaz:"
                    ),
                    t("Goal", "Amaç"),
                    self.dialogs.script_goal.trim()
                );
                self.dialogs.script_goal.clear();
                self.chat.mode = crate::ai::agent::Mode::Fast;
                let ctx = ui.ctx().clone();
                self.send_chat(&ctx);
            }
        });
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let r = ui.add(
                    egui::TextEdit::multiline(&mut code)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(14)
                        .hint_text("fn update(dt) {\n    this.ry += 90.0 * dt;\n}"),
                );
                if r.changed() {
                    if !self.editing {
                        self.push_undo();
                        self.editing = true;
                    }
                    if let Some(e) = self.scene.get_mut(id) {
                        e.script = code;
                    }
                    self.dirty = true;
                }
                if r.lost_focus() {
                    self.editing = false;
                }
            });
    }
}

fn list_rel(root: &std::path::Path, folder: &str, exts: &[&str]) -> Vec<String> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(root.join(folder)) {
        for e in rd.flatten() {
            let p = e.path();
            let ext = p
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_lowercase();
            if exts.contains(&ext.as_str()) {
                out.push(format!("{folder}/{}", e.file_name().to_string_lossy()));
            }
        }
    }
    out.sort();
    out
}

/// Agent memory settings (shared by Project Settings and the AI settings window).
pub fn memory_settings(ui: &mut egui::Ui, settings: &mut Settings, status: &Arc<Mutex<String>>) {
    ui.label(RichText::new(t("Agent memory", "Ajan hafızası")).strong());
    let m = &mut settings.memory;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut m.backend, MemoryBackend::Off, t("Off", "Kapalı"));
        ui.selectable_value(
            &mut m.backend,
            MemoryBackend::Builtin,
            t("Built-in (offline)", "Yerleşik (çevrimdışı)"),
        );
        ui.selectable_value(
            &mut m.backend,
            MemoryBackend::TencentDb,
            "TencentDB Agent Memory",
        );
    });
    if m.backend == MemoryBackend::TencentDb {
        egui::Grid::new("mem").num_columns(2).show(ui, |ui| {
            ui.label("Memory Core URL");
            ui.text_edit_singleline(&mut m.url);
            ui.end_row();
            ui.label("Token (KERNEL_AUTH_TOKEN)");
            ui.add(egui::TextEdit::singleline(&mut m.token).password(true));
            ui.end_row();
            ui.label("Service / Team");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut m.service_id).desired_width(90.0));
                ui.add(egui::TextEdit::singleline(&mut m.team_id).desired_width(90.0));
            });
            ui.end_row();
            ui.label("User / Agent");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut m.user_id).desired_width(90.0));
                ui.add(egui::TextEdit::singleline(&mut m.agent_id).desired_width(90.0));
            });
            ui.end_row();
        });
        ui.hyperlink_to(
            t(
                "Run it: github.com/TencentCloud/TencentDB-Agent-Memory (./start-all.sh)",
                "Kurulum: github.com/TencentCloud/TencentDB-Agent-Memory (./start-all.sh)",
            ),
            "https://github.com/TencentCloud/TencentDB-Agent-Memory",
        );
    }
    ui.horizontal(|ui| {
        ui.label(t("Recall", "Hatırlama"));
        ui.add(egui::DragValue::new(&mut m.recall_limit).range(1..=20));
        if ui.button(t("Test", "Test et")).clicked() {
            let mem = Memory::new(m.clone());
            let st = status.clone();
            let ctx = ui.ctx().clone();
            *st.lock().unwrap() = "…".into();
            std::thread::spawn(move || {
                *st.lock().unwrap() = match mem.health() {
                    Ok(s) => format!("✔ {s}"),
                    Err(e) => format!("✖ {e}"),
                };
                ctx.request_repaint();
            });
        }
        if m.backend == MemoryBackend::Builtin && ui.button(t("Clear", "Temizle")).clicked() {
            Memory::new(m.clone()).clear_local();
            *status.lock().unwrap() = t("cleared", "temizlendi").into();
        }
        ui.label(RichText::new(status.lock().unwrap().clone()).small());
    });
}
