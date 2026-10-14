//! Animation tab: clip editor with a keyframe timeline, preview and recording.

use super::{Editor, LogLevel};
use crate::animation::{self, Clip, Property};
use crate::i18n::t;
use crate::scene::{Animator, EntityId, Transform};
use crate::theme::{self, ACCENT};
use egui::{Color32, RichText};

pub struct AnimEditor {
    pub clip: Clip,
    /// Project-relative path of the clip being edited (None = unsaved).
    pub path: Option<String>,
    pub time: f32,
    pub playing: bool,
    /// Rest pose of the previewed entity.
    base: Option<(EntityId, Transform, [f32; 3])>,
    last: Option<std::time::Instant>,
    selected_key: Option<(Property, usize)>,
}

impl Default for AnimEditor {
    fn default() -> Self {
        Self {
            clip: animation::preset("bob").unwrap(),
            path: None,
            time: 0.0,
            playing: false,
            base: None,
            last: None,
            selected_key: None,
        }
    }
}

impl Editor {
    fn anim_restore(&mut self) {
        if let Some((id, tf, color)) = self.anim.base.take() {
            if let Some(e) = self.scene.get_mut(id) {
                e.transform = tf;
                e.color = color;
            }
        }
    }

    fn anim_apply(&mut self, id: EntityId) {
        if self.anim.base.as_ref().map_or(true, |b| b.0 != id) {
            self.anim_restore();
            if let Some(e) = self.scene.get(id) {
                self.anim.base = Some((id, e.transform.clone(), e.color));
            }
        }
        let Some((_, base, color)) = self.anim.base.clone() else {
            return;
        };
        let pose = self.anim.clip.sample(self.anim.time);
        if let Some(e) = self.scene.get_mut(id) {
            e.transform.position = base.position + pose.position;
            e.transform.rotation = base.rotation + pose.rotation;
            e.transform.scale = base.scale * pose.scale;
            e.color = pose.color.unwrap_or(color);
        }
    }

    pub fn animation_clips(&self) -> Vec<String> {
        let mut v: Vec<String> = animation::PRESETS
            .iter()
            .map(|p| format!("preset:{p}"))
            .collect();
        if let Ok(rd) = std::fs::read_dir(self.project_root().join("Animations")) {
            let mut files: Vec<String> = rd
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".anim.json"))
                .map(|e| format!("Animations/{}", e.file_name().to_string_lossy()))
                .collect();
            files.sort();
            v.extend(files);
        }
        v
    }

    pub fn animation_ui(&mut self, ui: &mut egui::Ui) {
        let sel = self.selected.filter(|s| self.scene.get(*s).is_some());
        // ------------------------------------------------ toolbar
        ui.horizontal_wrapped(|ui| {
            let current = self
                .anim
                .path
                .clone()
                .unwrap_or_else(|| format!("({})", self.anim.clip.name));
            egui::ComboBox::from_id_salt("clip_pick")
                .selected_text(current)
                .width(220.0)
                .show_ui(ui, |ui| {
                    for c in self.animation_clips() {
                        if ui
                            .selectable_label(self.anim.path.as_deref() == Some(c.as_str()), &c)
                            .clicked()
                        {
                            let loaded = match c.strip_prefix("preset:") {
                                Some(p) => {
                                    animation::preset(p).ok_or_else(|| "unknown preset".to_string())
                                }
                                None => Clip::load(&self.project_root().join(&c)),
                            };
                            match loaded {
                                Ok(clip) => {
                                    self.anim.clip = clip;
                                    self.anim.path =
                                        (!c.starts_with("preset:")).then_some(c.clone());
                                    self.anim.time = 0.0;
                                }
                                Err(e) => self.log(LogLevel::Error, e),
                            }
                        }
                    }
                });
            if ui.button(t("+ New", "+ Yeni")).clicked() {
                self.anim.clip = Clip::default();
                self.anim.path = None;
                self.anim.time = 0.0;
            }
            if ui.button(t("💾 Save", "💾 Kaydet")).clicked() {
                let rel = self.anim.path.clone().unwrap_or_else(|| {
                    let safe: String = self
                        .anim
                        .clip
                        .name
                        .chars()
                        .map(|c| if c.is_alphanumeric() { c } else { '_' })
                        .collect();
                    format!("Animations/{safe}.anim.json")
                });
                match self.anim.clip.save(&self.project_root().join(&rel)) {
                    Ok(()) => {
                        self.log(
                            LogLevel::Info,
                            format!("{} {rel}", t("Saved", "Kaydedildi:")),
                        );
                        self.anim.path = Some(rel);
                    }
                    Err(e) => self.log(LogLevel::Error, e),
                }
            }
            let can_assign = sel.is_some() && self.anim.path.is_some();
            if ui
                .add_enabled(
                    can_assign,
                    egui::Button::new(t("Assign to selected", "Seçiliye ata")),
                )
                .on_disabled_hover_text(t(
                    "Save the clip and select an object first.",
                    "Önce klibi kaydet ve bir nesne seç.",
                ))
                .clicked()
            {
                let clip = self.anim.path.clone().unwrap();
                self.anim_restore();
                self.push_undo();
                if let Some(e) = sel.and_then(|id| self.scene.get_mut(id)) {
                    e.animator = Some(Animator {
                        clip,
                        ..Default::default()
                    });
                }
            }
            ui.separator();
            ui.label(t("Name", "Ad"));
            ui.add(egui::TextEdit::singleline(&mut self.anim.clip.name).desired_width(110.0));
            ui.label(t("Length", "Süre"));
            ui.add(
                egui::DragValue::new(&mut self.anim.clip.length)
                    .speed(0.05)
                    .range(0.1..=120.0)
                    .suffix(" s"),
            );
            ui.checkbox(&mut self.anim.clip.looping, t("Loop", "Döngü"));
        });
        ui.separator();

        // ------------------------------------------------ transport
        let Some(id) = sel else {
            self.anim_restore();
            self.anim.playing = false;
            ui.label(
                RichText::new(t(
                    "Select an object to preview and record keyframes.",
                    "Önizlemek ve anahtar kare kaydetmek için bir nesne seç.",
                ))
                .weak(),
            );
            self.timeline(ui);
            return;
        };
        let name = self
            .scene
            .get(id)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("🎞 {name}")).strong());
            let play = if self.anim.playing { "⏸" } else { "▶" };
            if ui.button(play).clicked() {
                self.anim.playing = !self.anim.playing;
                self.anim.last = None;
            }
            if ui
                .button("■")
                .on_hover_text(t("Stop and restore pose", "Durdur ve pozu geri yükle"))
                .clicked()
            {
                self.anim.playing = false;
                self.anim.time = 0.0;
                self.anim_restore();
            }
            let len = self.anim.clip.length;
            if ui
                .add(
                    egui::Slider::new(&mut self.anim.time, 0.0..=len)
                        .suffix(" s")
                        .max_decimals(2),
                )
                .changed()
            {
                self.anim.playing = false;
                self.anim_apply(id);
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(t(
                    "Record key from current pose:",
                    "Mevcut pozdan anahtar kaydet:",
                ))
                .size(11.5)
                .weak(),
            );
            for p in Property::ALL {
                if ui.button(format!("♦ {}", p.label())).clicked() {
                    if self.anim.base.as_ref().map_or(true, |b| b.0 != id) {
                        self.anim_apply(id);
                    }
                    let (_, base, _) = self.anim.base.clone().unwrap();
                    if let Some(e) = self.scene.get(id) {
                        let v = match p {
                            Property::Position => (e.transform.position - base.position).to_array(),
                            Property::Rotation => (e.transform.rotation - base.rotation).to_array(),
                            Property::Scale => (e.transform.scale
                                / base.scale.max(glam::Vec3::splat(1e-4)))
                            .to_array(),
                            Property::Color => e.color,
                        };
                        self.anim.clip.set_key(p, self.anim.time, v);
                    }
                }
            }
        });
        if self.anim.playing {
            let now = std::time::Instant::now();
            let dt = self
                .anim
                .last
                .map(|l| now.duration_since(l).as_secs_f32())
                .unwrap_or(0.0);
            self.anim.last = Some(now);
            self.anim.time += dt;
            if self.anim.time > self.anim.clip.length {
                if self.anim.clip.looping {
                    self.anim.time = self.anim.time.rem_euclid(self.anim.clip.length);
                } else {
                    self.anim.time = self.anim.clip.length;
                    self.anim.playing = false;
                }
            }
            self.anim_apply(id);
            ui.ctx().request_repaint();
        }
        self.timeline(ui);
    }

    fn timeline(&mut self, ui: &mut egui::Ui) {
        let len = self.anim.clip.length.max(0.01);
        let label_w = 90.0;
        let mut remove: Option<(Property, usize)> = None;
        for p in Property::ALL {
            let keys: Vec<(f32, [f32; 3])> = self
                .anim
                .clip
                .tracks
                .iter()
                .find(|tr| tr.property == p)
                .map(|tr| tr.keys.iter().map(|k| (k.t, k.v)).collect())
                .unwrap_or_default();
            ui.horizontal(|ui| {
                ui.add_sized(
                    [label_w, 22.0],
                    egui::Label::new(RichText::new(p.label()).size(12.0)),
                );
                let (rect, resp) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width() - 8.0, 22.0),
                    egui::Sense::click(),
                );
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 3.0, theme::BG0);
                for i in 0..=10 {
                    let x = rect.left() + rect.width() * i as f32 / 10.0;
                    painter.line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(1.0_f32, theme::BG2),
                    );
                }
                let head_x = rect.left() + rect.width() * (self.anim.time / len).clamp(0.0, 1.0);
                painter.line_segment(
                    [
                        egui::pos2(head_x, rect.top()),
                        egui::pos2(head_x, rect.bottom()),
                    ],
                    egui::Stroke::new(1.5_f32, ACCENT),
                );
                for (i, (kt, v)) in keys.iter().enumerate() {
                    let x = rect.left() + rect.width() * (kt / len).clamp(0.0, 1.0);
                    let c = rect.center().y;
                    let sel = self.anim.selected_key == Some((p, i));
                    let col = if sel {
                        theme::GOLD
                    } else {
                        Color32::from_gray(210)
                    };
                    let d = 5.0;
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(x, c - d),
                            egui::pos2(x + d, c),
                            egui::pos2(x, c + d),
                            egui::pos2(x - d, c),
                        ],
                        col,
                        egui::Stroke::NONE,
                    ));
                    let _ = v;
                }
                if resp.clicked() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let tt = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * len;
                        // Select a key near the click, otherwise move the playhead.
                        let near = keys.iter().position(|(kt, _)| {
                            ((kt / len) * rect.width() - (pos.x - rect.left())).abs() < 7.0
                        });
                        match near {
                            Some(i) => {
                                self.anim.selected_key = Some((p, i));
                                self.anim.time = keys[i].0;
                            }
                            None => {
                                self.anim.time = tt;
                                self.anim.selected_key = None;
                            }
                        }
                        self.anim.playing = false;
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            if let Some((p, i)) = self.anim.selected_key {
                if let Some(k) = self.anim.clip.tracks.iter_mut().find(|tr| tr.property == p).and_then(|tr| tr.keys.get_mut(i)) {
                    ui.label(RichText::new(format!("♦ {} @", p.label())).size(11.5));
                    ui.add(egui::DragValue::new(&mut k.t).speed(0.01).range(0.0..=len).suffix(" s"));
                    for c in 0..3 {
                        ui.add(egui::DragValue::new(&mut k.v[c]).speed(0.02).max_decimals(3));
                    }
                    if ui.button(t("🗑 Delete key", "🗑 Anahtarı sil")).clicked() {
                        remove = Some((p, i));
                    }
                }
            } else {
                ui.label(RichText::new(t("Click a ♦ to edit it; click the timeline to move the playhead.", "Düzenlemek için ♦'e tıkla; oynatma kafasını taşımak için zaman çizelgesine tıkla.")).size(11.0).weak());
            }
        });
        if let Some((p, i)) = remove {
            if let Some(tr) = self.anim.clip.tracks.iter_mut().find(|tr| tr.property == p) {
                if i < tr.keys.len() {
                    tr.keys.remove(i);
                }
            }
            self.anim.selected_key = None;
        }
    }
}
