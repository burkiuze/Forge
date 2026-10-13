//! Project folders, procedural starter textures and content-browser helpers.

use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};

pub const FOLDERS: [&str; 3] = ["Scripts", "Textures", "Prefabs"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AssetKind {
    Folder,
    Scene,
    Script,
    Texture,
    Prefab,
    Other,
}

impl AssetKind {
    pub fn of(p: &Path) -> Self {
        if p.is_dir() {
            return AssetKind::Folder;
        }
        let n = p
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if n.ends_with(".prefab.json") {
            AssetKind::Prefab
        } else if n.ends_with(".scene.json") || n.ends_with(".json") {
            AssetKind::Scene
        } else if n.ends_with(".rhai") {
            AssetKind::Script
        } else if n.ends_with(".png") || n.ends_with(".jpg") || n.ends_with(".jpeg") {
            AssetKind::Texture
        } else {
            AssetKind::Other
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            AssetKind::Folder => "📁",
            AssetKind::Scene => "🎬",
            AssetKind::Script => "📜",
            AssetKind::Texture => "🖼",
            AssetKind::Prefab => "📦",
            AssetKind::Other => "📄",
        }
    }
}

/// Make sure a project folder has the standard sub-folders and starter textures.
pub fn ensure_project(root: &Path) {
    for f in FOLDERS {
        let _ = std::fs::create_dir_all(root.join(f));
    }
    let tex = root.join("Textures");
    let gens: [(&str, fn(u32, u32) -> Rgba<u8>); 5] = [
        ("checker.png", checker),
        ("bricks.png", bricks),
        ("planks.png", planks),
        ("tiles.png", tiles),
        ("grass.png", grass),
    ];
    for (name, f) in gens {
        let p = tex.join(name);
        if !p.exists() {
            let img = RgbaImage::from_fn(256, 256, f);
            let _ = img.save(&p);
        }
    }
}

pub fn list_dir(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    !p.file_name()
                        .map_or(true, |n| n.to_string_lossy().starts_with('.'))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|p| {
        (
            !p.is_dir(),
            p.file_name().map(|n| n.to_string_lossy().to_lowercase()),
        )
    });
    v
}

fn hash(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    (h ^ (h >> 16)) as f32 / u32::MAX as f32
}

fn shade(c: [f32; 3], k: f32) -> Rgba<u8> {
    Rgba([
        (c[0] * k * 255.0).clamp(0.0, 255.0) as u8,
        (c[1] * k * 255.0).clamp(0.0, 255.0) as u8,
        (c[2] * k * 255.0).clamp(0.0, 255.0) as u8,
        255,
    ])
}

fn checker(x: u32, y: u32) -> Rgba<u8> {
    let k = if (x / 32 + y / 32) % 2 == 0 {
        0.92
    } else {
        0.62
    };
    shade([1.0, 1.0, 1.0], k)
}

fn bricks(x: u32, y: u32) -> Rgba<u8> {
    let row = y / 32;
    let off = if row % 2 == 0 { 0 } else { 32 };
    let bx = (x + off) % 64;
    let by = y % 32;
    let mortar = bx < 3 || by < 3;
    if mortar {
        shade([0.78, 0.76, 0.72], 0.9)
    } else {
        let n = hash((x + off) / 64, row) * 0.25 + hash(x, y) * 0.08;
        shade([0.72, 0.33, 0.22], 0.8 + n)
    }
}

fn planks(x: u32, y: u32) -> Rgba<u8> {
    let plank = x / 42;
    let gap = x % 42 < 2;
    let grain = ((y as f32 * 0.09 + hash(plank, 0) * 40.0).sin() * 0.5 + 0.5) * 0.12;
    if gap {
        shade([0.35, 0.22, 0.12], 0.8)
    } else {
        shade([0.62, 0.42, 0.24], 0.85 + grain + hash(plank, 7) * 0.12)
    }
}

fn tiles(x: u32, y: u32) -> Rgba<u8> {
    let edge = x % 64 < 2 || y % 64 < 2;
    if edge {
        shade([0.3, 0.3, 0.32], 1.0)
    } else {
        shade([0.82, 0.84, 0.86], 0.92 + hash(x / 64, y / 64) * 0.08)
    }
}

fn grass(x: u32, y: u32) -> Rgba<u8> {
    let n = hash(x, y) * 0.35 + hash(x / 4, y / 4) * 0.25 + hash(x / 16, y / 16) * 0.2;
    shade([0.28, 0.55, 0.22], 0.65 + n)
}
