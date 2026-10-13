#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Forge — AI-native game engine & editor.

mod ai;
mod app;
mod assets;
mod catalog;
mod chat;
mod config;
mod i18n;
mod library;
mod mesh;
mod model;
mod render;
mod runtime;
mod scene;
mod start;
mod store;
mod templates;
mod theme;
mod updater;
mod wizard;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

fn icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/logo.png"))
        .expect("logo")
        .to_rgba8();
    let (w, h) = img.dimensions();
    egui::IconData {
        rgba: img.into_raw(),
        width: w,
        height: h,
    }
}

/// `forge --play game.scene.json`, or an exported game (a `game.scene.json` next to the executable).
fn player_scene() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--play") {
        return args.get(i + 1).map(PathBuf::from);
    }
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.join("game.scene.json");
    p.is_file().then_some(p)
}

fn options(msaa: u16, player: bool) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Forge")
            .with_app_id("forge")
            .with_inner_size([
                if player { 1280.0 } else { 1600.0 },
                if player { 720.0 } else { 940.0 },
            ])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(icon()),
        depth_buffer: 24,
        multisampling: msaa,
        vsync: true,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    }
}

fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("forge {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let player = player_scene();
    let run = |msaa: u16| {
        let p = player.clone();
        eframe::run_native(
            "Forge",
            options(msaa, p.is_some()),
            Box::new(move |cc| Ok(Box::new(app::ForgeApp::new(cc, p)))),
        )
    };
    // MSAA is nice-to-have; retry without it on drivers that refuse.
    if let Err(e) = run(4) {
        eprintln!("Forge: retrying without MSAA ({e})");
        if let Err(e) = run(0) {
            eprintln!("Forge failed to start: {e}");
            std::process::exit(1);
        }
    }
}
