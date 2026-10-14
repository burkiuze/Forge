//! Model registry: OpenRouter cloud models + local (llama.cpp / Ollama) models.

use crate::config::{Settings, LOCAL_LLAMA_PORT, OPENROUTER_BASE};

/// OpenRouter's free-model router: picks an available free model per request.
pub const AUTO_ID: &str = "openrouter/free";

pub struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
}

/// What "Auto" really sends: free models that follow tool instructions, tried in order
/// (OpenRouter falls through on rate limits). `openrouter/free` alone can land on safety
/// classifiers or tiny models that can't build anything.
pub const AUTO_MODELS: &[&str] = &[
    "nvidia/nemotron-3-super-120b-a12b:free",
    "nvidia/nemotron-3-ultra-550b-a55b:free",
    "google/gemma-4-31b-it:free",
];

/// Free models that fail inside Forge (not chat models, agent-harness only, or empty replies).
/// Saved settings that point at them fall back to Auto.
pub const RETIRED_MODELS: &[&str] = &[
    "inception/mercury-decide:free",
    "thinkingmachines/inkling-small:free",
    "cohere/north-mini-code:free",
];

/// Built-in cloud models (all free tier on OpenRouter).
pub const CLOUD_MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: AUTO_ID,
        name: "Auto",
        blurb: "Best free models, switches automatically when one is busy",
    },
    ModelInfo {
        id: "nvidia/nemotron-3-super-120b-a12b:free",
        name: "Nemotron 3 Super",
        blurb: "NVIDIA · 120B MoE · 262K ctx · strong agent",
    },
    ModelInfo {
        id: "nvidia/nemotron-3-ultra-550b-a55b:free",
        name: "Nemotron 3 Ultra",
        blurb: "NVIDIA · 550B MoE · 1M ctx",
    },
    ModelInfo {
        id: "google/gemma-4-31b-it:free",
        name: "Gemma 4 31B",
        blurb: "Google DeepMind · 262K ctx · multimodal, tool use",
    },
];

#[derive(Clone, Debug, PartialEq)]
pub enum Backend {
    OpenRouter,
    /// GGUF served by Forge-managed llama-server.
    Llama,
    Ollama,
}

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub backend: Backend,
    pub base: String,
    pub key: String,
    pub model: String,
}

/// Model ids: `vendor/model[:free]` → OpenRouter, `local:<file.gguf>` → llama.cpp, `ollama:<tag>` → Ollama.
pub fn resolve(id: &str, settings: &Settings, openrouter_key: &str) -> Endpoint {
    if let Some(file) = id.strip_prefix("local:") {
        Endpoint {
            backend: Backend::Llama,
            base: format!("http://127.0.0.1:{LOCAL_LLAMA_PORT}/v1"),
            key: String::new(),
            model: file.to_string(),
        }
    } else if let Some(tag) = id.strip_prefix("ollama:") {
        Endpoint {
            backend: Backend::Ollama,
            base: settings.ollama_url.trim_end_matches('/').to_string(),
            key: String::new(),
            model: tag.to_string(),
        }
    } else {
        Endpoint {
            backend: Backend::OpenRouter,
            base: OPENROUTER_BASE.into(),
            key: openrouter_key.to_string(),
            model: id.to_string(),
        }
    }
}

pub fn display_name(id: &str) -> String {
    if let Some(m) = CLOUD_MODELS.iter().find(|m| m.id == id) {
        return m.name.to_string();
    }
    if let Some(f) = id.strip_prefix("local:") {
        return format!("💻 {}", f.trim_end_matches(".gguf"));
    }
    if let Some(f) = id.strip_prefix("ollama:") {
        return format!("Ollama · {f}");
    }
    id.to_string()
}
