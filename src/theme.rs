//! Forge look & feel — dark charcoal with the logo's ember orange.

use egui::{Color32, CornerRadius, Stroke, Visuals};

pub const ACCENT: Color32 = Color32::from_rgb(240, 90, 30);
pub const GOLD: Color32 = Color32::from_rgb(246, 168, 33);
pub const BONE: Color32 = Color32::from_rgb(226, 224, 219);
pub const BG0: Color32 = Color32::from_rgb(20, 20, 20);
pub const BG1: Color32 = Color32::from_rgb(28, 28, 29);
pub const BG2: Color32 = Color32::from_rgb(38, 38, 40);
pub const BG3: Color32 = Color32::from_rgb(52, 52, 55);

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = BG1;
    v.window_fill = BG1;
    v.extreme_bg_color = BG0;
    v.faint_bg_color = BG2;
    v.code_bg_color = BG0;
    v.override_text_color = Some(BONE);
    v.hyperlink_color = GOLD;
    v.selection.bg_fill = ACCENT.linear_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.window_stroke = Stroke::new(1.0_f32, BG3);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(5);
    }
    v.widgets.noninteractive.bg_fill = BG1;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BG3);
    v.widgets.inactive.bg_fill = BG2;
    v.widgets.inactive.weak_bg_fill = BG2;
    v.widgets.hovered.bg_fill = BG3;
    v.widgets.hovered.weak_bg_fill = BG3;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT.linear_multiply(0.7));
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.weak_bg_fill = ACCENT.linear_multiply(0.8);
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.spacing.interact_size.y = 22.0;
    });
}

/// The Forge "F" mark, drawn as vector shapes (matches the app logo).
pub fn draw_logo(painter: &egui::Painter, rect: egui::Rect) {
    let s = rect.width().min(rect.height());
    let o = rect.center() - egui::vec2(s * 0.5, s * 0.5);
    let p = |x: f32, y: f32| o + egui::vec2(x * s, y * s);
    // Orange top bar (slanted right edge).
    painter.add(egui::Shape::convex_polygon(
        vec![p(0.10, 0.22), p(0.80, 0.22), p(0.70, 0.40), p(0.10, 0.40)],
        ACCENT,
        Stroke::NONE,
    ));
    // Stem.
    painter.rect_filled(egui::Rect::from_min_max(p(0.10, 0.46), p(0.27, 0.88)), 0.0, BONE);
    // Mid bar.
    painter.add(egui::Shape::convex_polygon(
        vec![p(0.33, 0.46), p(0.62, 0.46), p(0.54, 0.62), p(0.33, 0.62)],
        BONE,
        Stroke::NONE,
    ));
    // Sparks.
    for (x, y, r) in [(0.84, 0.14, 0.045), (0.93, 0.27, 0.035), (0.83, 0.33, 0.022)] {
        let c = p(x, y);
        let d = r * s;
        painter.add(egui::Shape::convex_polygon(
            vec![c + egui::vec2(0.0, -d), c + egui::vec2(d, 0.0), c + egui::vec2(0.0, d), c + egui::vec2(-d, 0.0)],
            GOLD,
            Stroke::NONE,
        ));
    }
}
