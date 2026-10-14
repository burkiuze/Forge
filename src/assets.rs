//! Project folders, procedural starter textures and content-browser helpers.

use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};

pub const FOLDERS: [&str; 6] = [
    "Scripts",
    "Textures",
    "Prefabs",
    "Models",
    "Audio",
    "Animations",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AssetKind {
    Folder,
    Scene,
    Script,
    Texture,
    Prefab,
    Model,
    Audio,
    Animation,
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
        if n.ends_with(".anim.json") {
            AssetKind::Animation
        } else if crate::audio::is_audio(p) {
            AssetKind::Audio
        } else if n.ends_with(".prefab.json") {
            AssetKind::Prefab
        } else if n.ends_with(".scene.json") || n.ends_with(".json") {
            AssetKind::Scene
        } else if n.ends_with(".gltf") || n.ends_with(".glb") {
            AssetKind::Model
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
            AssetKind::Model => "🗿",
            AssetKind::Audio => "🔊",
            AssetKind::Animation => "🎞",
            AssetKind::Other => "📄",
        }
    }
}

/// Destination folder for an imported file, by kind.
pub fn folder_for(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Texture => "Textures",
        AssetKind::Model => "Models",
        AssetKind::Audio => "Audio",
        AssetKind::Animation => "Animations",
        AssetKind::Script => "Scripts",
        AssetKind::Prefab => "Prefabs",
        _ => "",
    }
}

/// Copy files from anywhere on disk into the project (sorted by type). A `.gltf` brings its
/// whole folder along (buffers/textures). Returns the project-relative paths.
pub fn import(root: &Path, files: &[PathBuf]) -> Vec<Result<String, String>> {
    files
        .iter()
        .map(|src| {
            let kind = AssetKind::of(src);
            let name = src
                .file_name()
                .ok_or("bad path")?
                .to_string_lossy()
                .to_string();
            if kind == AssetKind::Folder {
                return Err(format!("{name}: folders are not imported"));
            }
            let folder = folder_for(kind);
            let is_gltf = name.to_lowercase().ends_with(".gltf");
            if is_gltf {
                // Copy sibling files (bin, textures) into Models/<stem>/.
                let stem = src
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let dst_dir = root.join(folder).join(&stem);
                copy_dir(src.parent().ok_or("bad path")?, &dst_dir, 2)
                    .map_err(|e| e.to_string())?;
                return Ok(format!("{folder}/{stem}/{name}"));
            }
            let dst = if folder.is_empty() {
                root.join(&name)
            } else {
                root.join(folder).join(&name)
            };
            if let Some(d) = dst.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            std::fs::copy(src, &dst).map_err(|e| format!("{name}: {e}"))?;
            Ok(if folder.is_empty() {
                name
            } else {
                format!("{folder}/{name}")
            })
        })
        .collect()
}

fn copy_dir(src: &Path, dst: &Path, depth: u32) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                copy_dir(&p, &dst.join(e.file_name()), depth - 1)?;
            }
        } else if std::fs::metadata(&p)
            .map(|m| m.len() < 512 << 20)
            .unwrap_or(false)
        {
            std::fs::copy(&p, dst.join(e.file_name()))?;
        }
    }
    Ok(())
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
