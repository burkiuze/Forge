//! Settings, paths and secret loading (`.env`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
pub const LOCAL_LLAMA_PORT: u16 = 8089;

/// Root data directory: `%APPDATA%\Forge` / `~/.local/share/Forge` / `~/Library/Application Support/Forge`.
pub fn data_dir() -> PathBuf {
    let dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Forge");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn env_file() -> PathBuf {
    data_dir().join(".env")
}

pub fn default_projects_dir() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(|| data_dir())
        .join("Forge Projects")
}

/// How agent actions are approved.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum Permission {
    /// Every change (scene edits, file writes, asset downloads) asks Accept / Reject.
    #[default]
    Manual,
    /// Everything is accepted automatically.
    Auto,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub setup_done: bool,
    pub models_dir: PathBuf,
    pub runtime_dir: PathBuf,
    /// Path to `llama-server` (llama.cpp). Empty = auto-detect in runtime_dir.
    pub llama_server: String,
    pub llama_ctx: u32,
    pub llama_gpu_layers: i32,
    pub ollama_url: String,
    pub selected_model: String,
    /// Per-agent model override (empty = use selected model).
    pub planner_model: String,
    pub builder_model: String,
    pub reviewer_model: String,
    pub coder_model: String,
    /// Optional LLM router (e.g. Mercury Decide). Empty = fast heuristic.
    pub router_model: String,
    pub use_planner: bool,
    pub use_reviewer: bool,
    pub max_agent_steps: u32,
    pub temperature: f32,
    pub last_scene: Option<PathBuf>,
    pub projects_dir: PathBuf,
    pub custom_models: Vec<String>,
    pub lang: crate::i18n::Lang,
    pub memory: crate::ai::memory::MemoryConfig,
    pub agent_web: bool,
    pub agent_mode: crate::ai::agent::Mode,
    pub snap: bool,
    pub snap_move: f32,
    pub snap_rotate: f32,
    pub snap_scale: f32,
    pub show_stats: bool,
    pub show_grid: bool,
    pub ai_enabled: bool,
    pub permission: Permission,
    pub autosave_minutes: u32,
    pub show_start_screen: bool,
    pub camera_speed: f32,
    pub gizmo_size: f32,
    pub show_icons: bool,
    pub master_volume: f32,
    pub mute_editor_audio: bool,
    pub auto_update_check: bool,
    pub ui_scale: f32,
    pub agent_context_turns: u32,
    pub web_results: u32,
}

impl Default for Settings {
    fn default() -> Self {
        let d = data_dir();
        Self {
            setup_done: false,
            models_dir: d.join("models"),
            runtime_dir: d.join("runtime"),
            llama_server: String::new(),
            llama_ctx: 8192,
            llama_gpu_layers: 99,
            ollama_url: "http://127.0.0.1:11434/v1".into(),
            selected_model: crate::ai::models::AUTO_ID.into(),
            planner_model: String::new(),
            builder_model: String::new(),
            reviewer_model: String::new(),
            coder_model: "cohere/north-mini-code:free".into(),
            router_model: String::new(),
            use_planner: true,
            use_reviewer: false,
            max_agent_steps: 8,
            temperature: 0.4,
            last_scene: None,
            projects_dir: default_projects_dir(),
            custom_models: vec![],
            lang: crate::i18n::detect(),
            memory: Default::default(),
            agent_web: true,
            agent_mode: Default::default(),
            snap: false,
            snap_move: 0.5,
            snap_rotate: 15.0,
            snap_scale: 0.1,
            show_stats: false,
            show_grid: true,
            ai_enabled: true,
            permission: Permission::Manual,
            autosave_minutes: 5,
            show_start_screen: true,
            camera_speed: 1.0,
            gizmo_size: 1.0,
            show_icons: true,
            master_volume: 1.0,
            mute_editor_audio: false,
            auto_update_check: true,
            ui_scale: 1.0,
            agent_context_turns: 6,
            web_results: 6,
        }
    }
}

impl Settings {
    fn path() -> PathBuf {
        data_dir().join("settings.json")
    }
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        let _ = std::fs::create_dir_all(&self.models_dir);
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), s);
        }
    }
}

/// Secrets. Looked up (first hit wins): process env, `<exe dir>/.env`,
/// `<cwd>/.env`, `<data dir>/.env`. The `Forge.env` repo holds the team's `.env`.
#[derive(Clone, Default)]
pub struct Secrets {
    pub openrouter_key: String,
    pub hf_token: String,
}

impl Secrets {
    pub fn load() -> Self {
        let mut candidates: Vec<PathBuf> = vec![];
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join(".env"));
                candidates.push(dir.join("Forge.env").join(".env"));
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join(".env"));
        }
        candidates.push(env_file());
        for c in candidates {
            if c.is_file() {
                let _ = dotenvy::from_path(&c);
            }
        }
        Self {
            openrouter_key: std::env::var("OPENROUTER_API_KEY").unwrap_or_default(),
            hf_token: std::env::var("HF_TOKEN").unwrap_or_default(),
        }
    }

    /// Persist secrets to the user's private `.env` in the data directory.
    pub fn save(&self) -> std::io::Result<()> {
        write_env(
            &env_file(),
            &[
                ("OPENROUTER_API_KEY", &self.openrouter_key),
                ("HF_TOKEN", &self.hf_token),
            ],
        )
    }
}

fn write_env(path: &Path, kv: &[(&str, &str)]) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|l| {
            !kv.iter()
                .any(|(k, _)| l.trim_start().starts_with(&format!("{k}=")))
        })
        .map(str::to_string)
        .collect();
    for (k, v) in kv {
        if !v.is_empty() {
            lines.push(format!("{k}={v}"));
        }
    }
    std::fs::write(path, lines.join("\n") + "\n")
}

pub fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} B")
    }
}
