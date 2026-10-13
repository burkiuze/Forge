//! Model registry: OpenRouter cloud models + local (llama.cpp / Ollama) models.

use crate::config::{Settings, LOCAL_LLAMA_PORT, OPENROUTER_BASE};

/// OpenRouter's free-model router: picks an available free model per request.
pub const AUTO_ID: &str = "openrouter/free";

pub struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
}

/// Built-in cloud models (all free tier on OpenRouter).
pub const CLOUD_MODELS: &[ModelInfo] = &[
    ModelInfo { id: AUTO_ID, name: "Auto (free router)", blurb: "OpenRouter picks the best available free model" },
    ModelInfo { id: "google/gemma-4-31b-it:free", name: "Gemma 4 31B", blurb: "Google DeepMind · 262K ctx · multimodal, tool use" },
    ModelInfo { id: "inception/mercury-decide:free", name: "Mercury Decide", blurb: "Inception · structured decisions (great as router)" },
    ModelInfo { id: "cohere/north-mini-code:free", name: "North Mini Code", blurb: "Cohere · agentic coding MoE · 256K ctx" },
    ModelInfo { id: "thinkingmachines/inkling-small:free", name: "Inkling Small", blurb: "Thinking Machines · MoE · 1.05M ctx" },
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
        Endpoint { backend: Backend::Ollama, base: settings.ollama_url.trim_end_matches('/').to_string(), key: String::new(), model: tag.to_string() }
    } else {
        Endpoint { backend: Backend::OpenRouter, base: OPENROUTER_BASE.into(), key: openrouter_key.to_string(), model: id.to_string() }
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
