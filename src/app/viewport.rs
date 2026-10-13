//! Scene / Game viewports: camera controls, picking, move/rotate/scale gizmos, toolbar, stats.

use super::{Editor, GizmoDrag, Tool};
use crate::i18n::t;
use crate::render::{self, CameraView, DrawList, Shading};
use crate::scene::{MeshKind, Scene};
use crate::theme::{self, ACCENT};
use eframe::egui_glow;
use egui::{Color32, RichText};
use glam::Vec3;
use std::sync::Arc;

const AXIS_COLORS: [Color32; 3] = [
    Color32::from_rgb(235, 75, 65),
    Color32::from_rgb(110, 210, 90),
    Color32::from_rgb(75, 140, 245),
];

/// A gizmo handle in screen space.
struct Handle {
    axis: usize,
    pts: Vec<egui::Pos2>,
    len: f32,
}

fn seg_dist(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

fn poly_dist(p: egui::Pos2, pts: &[egui::Pos2]) -> f32 {
    pts.windows(2)
        .map(|w| seg_dist(p, w[0], w[1]))
        .fold(f32::MAX, f32::min)
}

impl Editor {
    pub fn viewport_ui(&mut self, ui: &mut egui::Ui, game_view: bool) {
        let full = ui.available_rect_before_wrap();
        let rect = full;
        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let aspect = rect.width() / rect.height().max(1.0);
        let playing = self.runtime.is_some();

        let scene: &Scene = match &self.runtime {
            Some(rt) => &rt.scene,
            None => &self.scene,
        };
        let cam: CameraView = if game_view {
            render::main_camera(scene)
                .and_then(|c| render::entity_camera(scene, c, aspect))
                .unwrap_or_else(|| self.cam.view(aspect))
        } else {
            self.cam.view(aspect)
        };

        // ---------------------------------------------------------- editor camera controls
        if !game_view {
            let input = ui.input(|i| i.clone());
            if resp.hovered() {
                let scroll = input.smooth_scroll_delta.y;
                if scroll != 0.0 {
                    self.cam.zoom(scroll);
                }
            }
            let shift = input.modifiers.shift;
            let alt = input.modifiers.alt;
            if (resp.dragged_by(egui::PointerButton::Secondary) && !shift)
                || (alt && resp.dragged_by(egui::PointerButton::Primary))
            {
                let d = resp.drag_delta();
                self.cam.orbit(d.x, d.y);
            }
            if resp.dragged_by(egui::PointerButton::Middle)
                || (resp.dragged_by(egui::PointerButton::Secondary) && shift)
            {
                let d = resp.drag_delta();
                self.cam.pan(d.x, d.y);
            }
            if (input.pointer.secondary_down() && resp.hovered())
                || resp.dragged_by(egui::PointerButton::Secondary)
            {
                let k = |key| if input.key_down(key) { 1.0 } else { 0.0 };
                let mv = Vec3::new(
                    k(egui::Key::D) - k(egui::Key::A),
                    k(egui::Key::E) - k(egui::Key::Q),
                    k(egui::Key::W) - k(egui::Key::S),
                );
                if mv != Vec3::ZERO {
                    let boost = if shift { 3.0 } else { 1.0 };
                    self.cam.fly(mv * input.stable_dt.min(0.05) * 10.0 * boost);
                    ui.ctx().request_repaint();
                }
            }
            if resp.hovered() && input.key_pressed(egui::Key::F) && !ui.ctx().wants_keyboard_input()
            {
                if let Some(s) = self.selected {
                    self.focus(s);
                }
            }
        }

        // ---------------------------------------------------------- gizmo handles
        let mut handles: Vec<Handle> = vec![];
        let mut center_screen = None;
        if !game_view && !playing {
            if let Some(sel) = self.selected.filter(|s| self.scene.get(*s).is_some()) {
                let p = self.scene.world_matrix(sel).w_axis.truncate();
                let len = (cam.eye.distance(p) * 0.14).max(0.2);
                if let Some(sp) = cam.project(p, rect) {
                    center_screen = Some(sp);
                    for (i, axis) in [Vec3::X, Vec3::Y, Vec3::Z].iter().enumerate() {
                        match self.tool {
                            Tool::Move | Tool::Scale => {
                                if let Some(ep) = cam.project(p + *axis * len, rect) {
                                    handles.push(Handle {
                                        axis: i,
                                        pts: vec![sp, ep],
                                        len,
                                    });
                                }
                            }
                            Tool::Rotate => {
                                let (u, v) = match i {
                                    0 => (Vec3::Y, Vec3::Z),
                                    1 => (Vec3::Z, Vec3::X),
                                    _ => (Vec3::X, Vec3::Y),
                                };
                                let pts: Vec<egui::Pos2> = (0..=64)
                                    .filter_map(|k| {
                                        let a = k as f32 / 64.0 * std::f32::consts::TAU;
                                        cam.project(
                                            p + (u * a.cos() + v * a.sin()) * len * 0.9,
                                            rect,
                                        )
                                    })
                                    .collect();
                                if pts.len() > 8 {
                                    handles.push(Handle { axis: i, pts, len });
                                }
                            }
                        }
                    }
                }
            }
            let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
            if resp.drag_started_by(egui::PointerButton::Primary) && !ui.input(|i| i.modifiers.alt)
            {
                if let Some(mp) = pointer {
                    let mut hit = handles
                        .iter()
                        .map(|h| (h.axis, poly_dist(mp, &h.pts)))
                        .filter(|(_, d)| *d < 9.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|h| h.0);
                    if hit.is_none() && self.tool == Tool::Scale {
                        if let Some(c) = center_screen {
                            if (mp - c).length() < 9.0 {
                                hit = Some(3);
                            }
                        }
                    }
                    if let (Some(axis), Some(sel)) = (hit, self.selected) {
                        self.push_undo();
                        let start = self.scene.get(sel).unwrap().transform.clone();
                        let last_angle = center_screen.map(|c| (mp - c).angle()).unwrap_or(0.0);
                        self.gizmo = Some(GizmoDrag {
                            axis,
                            tool: self.tool,
                            start,
                            accumulated: 0.0,
                            last_angle,
                        });
                    }
                }
            }
            let snap = self.settings.snap || ui.input(|i| i.modifiers.ctrl);
            let (sm, sr, ss) = (
                self.settings.snap_move,
                self.settings.snap_rotate,
                self.settings.snap_scale,
            );
            if let (Some(g), Some(sel), Some(mp)) = (&mut self.gizmo, self.selected, pointer) {
                if resp.dragged_by(egui::PointerButton::Primary) {
                    let delta = resp.drag_delta();
                    let mut tf = g.start.clone();
                    match g.tool {
                        Tool::Move | Tool::Scale if g.axis < 3 => {
                            if let Some(h) = handles.iter().find(|h| h.axis == g.axis) {
                                let sv = h.pts[1] - h.pts[0];
                                let sl = sv.length().max(1.0);
                                g.accumulated += delta.dot(sv / sl) / sl * h.len;
                            }
                            let mut amt = g.accumulated;
                            if g.tool == Tool::Move {
                                if snap {
                                    amt = (amt / sm).round() * sm;
                                }
                                tf.position[g.axis] += amt;
                            } else {
                                let mut f = 1.0 + amt / g.start.scale[g.axis].abs().max(0.05);
                                if snap {
                                    let target = ((g.start.scale[g.axis] * f) / ss).round() * ss;
                                    f = target / g.start.scale[g.axis].max(1e-4);
                                }
                                tf.scale[g.axis] = (g.start.scale[g.axis] * f).max(0.01);
                            }
                        }
                        Tool::Move | Tool::Scale => {
                            g.accumulated += -delta.y * 0.01;
                            let mut f = (1.0 + g.accumulated).max(0.01);
                            if snap {
                                f = (f / ss).round() * ss;
                            }
                            tf.scale = g.start.scale * f.max(0.01);
                        }
                        Tool::Rotate => {
                            if let Some(c) = center_screen {
                                let ang = (mp - c).angle();
                                let mut d = ang - g.last_angle;
                                if d > std::f32::consts::PI {
                                    d -= std::f32::consts::TAU;
                                } else if d < -std::f32::consts::PI {
                                    d += std::f32::consts::TAU;
                                }
                                g.last_angle = ang;
                                let axis = [Vec3::X, Vec3::Y, Vec3::Z][g.axis];
                                let towards = axis.dot(cam.eye - g.start.position) > 0.0;
                                g.accumulated += d.to_degrees() * if towards { -1.0 } else { 1.0 };
                            }
                            let mut amt = g.accumulated;
                            if snap {
                                amt = (amt / sr).round() * sr;
                            }
                            tf.rotation[g.axis] += amt;
                        }
                    }
                    if let Some(e) = self.scene.get_mut(sel) {
                        e.transform = tf;
                    }
                }
            }
            if resp.drag_stopped() {
                self.gizmo = None;
            }
            if resp.clicked() && self.gizmo.is_none() {
                if let Some(mp) = resp.interact_pointer_pos() {
                    let (o, d) = cam.ray(mp, rect);
                    self.selected = render::pick(&self.scene, o, d);
                }
            }
        }

        // ---------------------------------------------------------- 3D paint
        let scene: &Scene = match &self.runtime {
            Some(rt) => &rt.scene,
            None => &self.scene,
        };
        let sel = if game_view { None } else { self.selected };
        let root = self.project_root();
        let shading = if game_view {
            Shading::Lit
        } else {
            self.shading
        };
        let dl = DrawList::from_scene(
            scene,
            cam,
            sel,
            !game_view && self.settings.show_grid,
            Some(&root),
            shading,
        );
        let stats = dl.stats();
        if !game_view {
            self.stats = stats;
        }
        if let Some(r) = &self.renderer {
            let r = r.clone();
            let cb = egui_glow::CallbackFn::new(move |info, painter| {
                r.lock().unwrap().paint(painter.gl(), &dl, &info);
            });
            ui.painter().add(egui::PaintCallback {
                rect,
                callback: Arc::new(cb),
            });
        } else {
            ui.painter().rect_filled(rect, 0.0, theme::BG0);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "OpenGL 3.3 renderer unavailable",
                egui::FontId::proportional(16.0),
                Color32::LIGHT_RED,
            );
        }

        // ---------------------------------------------------------- overlays
        let has_cam = render::main_camera(scene).is_some();
        let mut icons = vec![];
        if !game_view {
            for e in &scene.entities {
                if e.mesh != MeshKind::None || !scene.is_active(e.id) {
                    continue;
                }
                let p = scene.world_matrix(e.id).w_axis.truncate();
                if let Some(sp) = cam.project(p, rect) {
                    let icon = if e.camera.is_some() {
                        "🎥"
                    } else if let Some(l) = &e.light {
                        if l.kind == crate::scene::LightKind::Point {
                            "💡"
                        } else {
                            "☀"
                        }
                    } else {
                        "◎"
                    };
                    let c = if Some(e.id) == self.selected {
                        ACCENT
                    } else {
                        theme::BONE
                    };
                    icons.push((sp, icon, c));
                }
            }
        }
        let painter = ui.painter_at(rect);
        if !game_view {
            for (sp, icon, c) in icons {
                painter.text(
                    sp,
                    egui::Align2::CENTER_CENTER,
                    icon,
                    egui::FontId::proportional(18.0),
                    c,
                );
            }
            for h in &handles {
                let c = AXIS_COLORS[h.axis];
                let active = self.gizmo.as_ref().map_or(false, |g| g.axis == h.axis);
                let w = if active { 4.0_f32 } else { 2.5 };
                match self.tool {
                    Tool::Move => {
                        painter.line_segment([h.pts[0], h.pts[1]], egui::Stroke::new(w, c));
                        let dir = (h.pts[1] - h.pts[0]).normalized();
                        let n = egui::vec2(-dir.y, dir.x);
                        let tip = h.pts[1] + dir * 12.0;
                        painter.add(egui::Shape::convex_polygon(
                            vec![tip, h.pts[1] + n * 5.0, h.pts[1] - n * 5.0],
                            c,
                            egui::Stroke::NONE,
                        ));
                    }
                    Tool::Scale => {
                        painter.line_segment([h.pts[0], h.pts[1]], egui::Stroke::new(w, c));
                        painter.rect_filled(
                            egui::Rect::from_center_size(h.pts[1], egui::vec2(10.0, 10.0)),
                            1.0,
                            c,
                        );
                    }
                    Tool::Rotate => {
                        painter.add(egui::Shape::line(h.pts.clone(), egui::Stroke::new(w, c)));
                    }
                }
            }
            if let (Some(c), Tool::Scale) = (center_screen, self.tool) {
                painter.rect_filled(
                    egui::Rect::from_center_size(c, egui::vec2(11.0, 11.0)),
                    2.0,
                    Color32::from_gray(220),
                );
            }
            if let (Some(c), Tool::Move | Tool::Rotate) = (center_screen, self.tool) {
                painter.circle_filled(c, 3.5, Color32::WHITE);
            }
            self.toolbar_overlay(ui, rect);
            if self.settings.show_stats {
                self.stats_overlay(&painter, rect);
            }
            // Orientation hint.
            painter.text(
                rect.right_top() + egui::vec2(-12.0, 46.0),
                egui::Align2::RIGHT_TOP,
                t("Persp", "Persp"),
                egui::FontId::proportional(11.0),
                Color32::from_gray(150),
            );
        }
        // HUD + play border.
        if let Some(rt) = &self.runtime {
            if game_view || !self.player_mode {
                let hud = rt.hud();
                if !hud.is_empty() {
                    let g = painter.layout_no_wrap(
                        hud,
                        egui::FontId::proportional(20.0),
                        Color32::WHITE,
                    );
                    let pos = rect.left_top() + egui::vec2(16.0, 14.0);
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            pos - egui::vec2(8.0, 4.0),
                            g.size() + egui::vec2(16.0, 8.0),
                        ),
                        6.0,
                        Color32::from_black_alpha(140),
                    );
                    painter.galley(pos, g, Color32::WHITE);
                }
            }
            if !self.player_mode {
                painter.rect_stroke(
                    rect.shrink(1.0),
                    0.0,
                    egui::Stroke::new(2.0_f32, if rt.paused { theme::GOLD } else { ACCENT }),
                    egui::StrokeKind::Inside,
                );
            }
            if game_view {
                resp.request_focus();
            }
        }
        if game_view && !has_cam {
            painter.text(
                rect.center_bottom() - egui::vec2(0.0, 18.0),
                egui::Align2::CENTER_CENTER,
                t(
                    "No camera in scene — showing editor view",
                    "Sahnede kamera yok — editör görünümü",
                ),
                egui::FontId::proportional(13.0),
                theme::GOLD,
            );
        }
        if game_view && !playing && !self.player_mode {
            painter.text(
                rect.center_top() + egui::vec2(0.0, 16.0),
                egui::Align2::CENTER_CENTER,
                t("Press ▶ to play", "Oynatmak için ▶"),
                egui::FontId::proportional(12.0),
                Color32::from_gray(170),
            );
        }
        // Drag & drop prefab/texture onto the scene view.
        if !game_view {
            if let Some(path) = resp.dnd_release_payload::<std::path::PathBuf>() {
                self.drop_asset(&path, resp.hover_pos().map(|mp| cam.ray(mp, rect)));
            }
        }
    }

    fn toolbar_overlay(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let area = egui::Rect::from_min_size(
            rect.left_top() + egui::vec2(8.0, 8.0),
            egui::vec2(rect.width() - 16.0, 26.0),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(area)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut child;
        let pill = |ui: &mut egui::Ui, label: &str, on: bool, hover: &str| -> bool {
            let b = egui::Button::new(RichText::new(label).size(12.0).color(if on {
                Color32::WHITE
            } else {
                theme::BONE
            }))
            .fill(if on {
                ACCENT
            } else {
                Color32::from_rgba_unmultiplied(28, 28, 30, 220)
            })
            .min_size(egui::vec2(24.0, 22.0));
            ui.add(b).on_hover_text(hover).clicked()
        };
        if pill(ui, "T", self.tool == Tool::Move, t("Move (W)", "Taşı (W)")) {
            self.tool = Tool::Move;
        }
        if pill(
            ui,
            "R",
            self.tool == Tool::Rotate,
            t("Rotate (E)", "Döndür (E)"),
        ) {
            self.tool = Tool::Rotate;
        }
        if pill(
            ui,
            "S",
            self.tool == Tool::Scale,
            t("Scale (R)", "Ölçekle (R)"),
        ) {
            self.tool = Tool::Scale;
        }
        ui.add_space(8.0);
        let shading = match self.shading {
            Shading::Lit => "Lit",
            Shading::Unlit => "Unlit",
            Shading::Wireframe => "Wireframe",
        };
        ui.menu_button(RichText::new(format!("🎨 {shading}")).size(12.0), |ui| {
            for (s, n) in [
                (Shading::Lit, "Lit"),
                (Shading::Unlit, "Unlit"),
                (Shading::Wireframe, "Wireframe"),
            ] {
                if ui.selectable_label(self.shading == s, n).clicked() {
                    self.shading = s;
                    ui.close_menu();
                }
            }
        });
        if pill(ui, "#", self.settings.show_grid, t("Grid", "Izgara")) {
            self.settings.show_grid = !self.settings.show_grid;
        }
        if pill(
            ui,
            t("Snap", "Yapış"),
            self.settings.snap,
            t(
                "Snap to grid / angle (or hold Ctrl)",
                "Izgaraya/açıya yapış (veya Ctrl)",
            ),
        ) {
            self.settings.snap = !self.settings.snap;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if pill(
                ui,
                t("› Stats", "› İstatistik"),
                self.settings.show_stats,
                "",
            ) {
                self.settings.show_stats = !self.settings.show_stats;
            }
            ui.label(
                RichText::new("3D")
                    .size(11.0)
                    .color(Color32::WHITE)
                    .background_color(Color32::from_rgb(60, 110, 200)),
            );
        });
    }

    fn stats_overlay(&self, painter: &egui::Painter, rect: egui::Rect) {
        let s = self.stats;
        let scripts = self
            .scene
            .entities
            .iter()
            .filter(|e| !e.script.is_empty())
            .count();
        let lines = [
            format!("FPS       {:.0}  ({:.1} ms)", self.fps, self.frame_ms),
            format!("Draw calls {}", s.draw_calls),
            format!("Triangles {}", s.triangles),
            format!("Objects   {}", self.scene.entities.len()),
            format!("Lights    {}", s.lights),
            format!("Scripts   {scripts}"),
            format!("RAM total {:.0} GB", self.library.ram_gb),
        ];
        let w = 190.0;
        let h = 12.0 + lines.len() as f32 * 15.0;
        let r = egui::Rect::from_min_size(
            rect.right_top() + egui::vec2(-w - 10.0, 40.0),
            egui::vec2(w, h),
        );
        painter.rect_filled(r, 6.0, Color32::from_black_alpha(170));
        for (i, l) in lines.iter().enumerate() {
            painter.text(
                r.left_top() + egui::vec2(10.0, 7.0 + i as f32 * 15.0),
                egui::Align2::LEFT_TOP,
                l,
                egui::FontId::monospace(11.0),
                theme::BONE,
            );
        }
    }

    /// Asset dropped on the scene view: prefab → instantiate at the hit point; texture → apply to the hit object.
    fn drop_asset(&mut self, path: &std::path::Path, ray: Option<(Vec3, Vec3)>) {
        use crate::assets::AssetKind;
        match AssetKind::of(path) {
            AssetKind::Prefab => {
                self.instantiate_prefab(path);
                if let (Some((o, d)), Some(sel)) = (ray, self.selected) {
                    if d.y.abs() > 1e-3 {
                        let tt = -o.y / d.y;
                        if tt > 0.0 {
                            let hit = o + d * tt;
                            if let Some(e) = self.scene.get_mut(sel) {
                                e.transform.position.x = hit.x;
                                e.transform.position.z = hit.z;
                            }
                        }
                    }
                }
            }
            AssetKind::Texture => {
                let target = ray
                    .and_then(|(o, d)| render::pick(&self.scene, o, d))
                    .or(self.selected);
                let rel = path
                    .strip_prefix(self.project_root())
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| path.display().to_string());
                if let Some(id) = target {
                    self.push_undo();
                    if let Some(e) = self.scene.get_mut(id) {
                        e.material.texture = rel;
                        e.color = [1.0, 1.0, 1.0];
                    }
                    self.selected = Some(id);
                }
            }
            AssetKind::Script => {
                if let (Some(id), Ok(code)) = (self.selected, std::fs::read_to_string(path)) {
                    self.push_undo();
                    if let Some(e) = self.scene.get_mut(id) {
                        e.script = code;
                    }
                }
            }
            AssetKind::Scene => self.open_scene(path.to_path_buf()),
            _ => {}
        }
    }
}
