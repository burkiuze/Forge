//! OpenAI-compatible chat client with SSE streaming (OpenRouter, llama.cpp, Ollama).

use super::models::Endpoint;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Msg {
    pub role: &'static str,
    pub content: String,
}

impl Msg {
    pub fn system(c: impl Into<String>) -> Self {
        Self {
            role: "system",
            content: c.into(),
        }
    }
    pub fn user(c: impl Into<String>) -> Self {
        Self {
            role: "user",
            content: c.into(),
        }
    }
    pub fn assistant(c: impl Into<String>) -> Self {
        Self {
            role: "assistant",
            content: c.into(),
        }
    }
}

pub struct Reply {
    pub text: String,
    /// Concrete model that answered (useful with `openrouter/free`).
    pub model: String,
}

/// TLS roots = Mozilla's webpki roots + the OS certificate store, so HTTPS works behind
/// corporate proxies / antivirus that inject their own CA.
pub fn tls() -> std::sync::Arc<rustls::ClientConfig> {
    static CFG: std::sync::OnceLock<std::sync::Arc<rustls::ClientConfig>> =
        std::sync::OnceLock::new();
    CFG.get_or_init(|| {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let native = rustls_native_certs::load_native_certs();
        roots.add_parsable_certificates(native.certs);
        let cfg = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("tls versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
        std::sync::Arc::new(cfg)
    })
    .clone()
}

/// Shared HTTP agent builder: system proxy (HTTPS_PROXY/ALL_PROXY) + combined TLS roots.
pub fn builder() -> ureq::AgentBuilder {
    ureq::AgentBuilder::new()
        .tls_config(tls())
        .try_proxy_from_env(true)
}

/// Should `url` bypass the proxy? Local services (llama.cpp, Ollama, Memory Core) and NO_PROXY hosts.
pub fn bypass_proxy(url: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?'])
        .next()
        .unwrap_or("");
    let host = host
        .rsplit_once(':')
        .filter(|(h, p)| !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        .map(|(h, _)| h)
        .unwrap_or(host);
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if host == "localhost" || host == "::1" || host.starts_with("127.") || host == "0.0.0.0" {
        return true;
    }
    let no_proxy = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default();
    no_proxy
        .split(',')
        .map(|e| {
            e.trim()
                .trim_start_matches('*')
                .trim_start_matches('.')
                .to_ascii_lowercase()
        })
        .filter(|e| !e.is_empty())
        .any(|e| e == "*" || host == e || host.ends_with(&format!(".{e}")))
}

/// Builder for a specific URL (proxy only when the host is not local / NO_PROXY).
pub fn builder_for(url: &str) -> ureq::AgentBuilder {
    if bypass_proxy(url) {
        ureq::AgentBuilder::new().tls_config(tls())
    } else {
        builder()
    }
}

/// Agent with the default timeouts for `url`.
pub fn agent_for(url: &str) -> ureq::Agent {
    builder_for(url)
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(180))
        .user_agent(concat!("Forge/", env!("CARGO_PKG_VERSION")))
        .build()
}

pub fn agent() -> ureq::Agent {
    builder()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(180))
        .user_agent(concat!("Forge/", env!("CARGO_PKG_VERSION")))
        .build()
}

pub fn stream_chat(
    ep: &Endpoint,
    messages: &[Msg],
    temperature: f32,
    cancel: &AtomicBool,
    mut on_delta: impl FnMut(&str),
) -> Result<Reply, String> {
    if ep.backend == super::models::Backend::OpenRouter && ep.key.trim().is_empty() {
        return Err(
            "OpenRouter API key missing — set it in AI › Settings (or OPENROUTER_API_KEY in .env)."
                .into(),
        );
    }
    let msgs: Vec<Value> = messages
        .iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
    let mut body = json!({
        "model": ep.model,
        "messages": msgs,
        "stream": true,
        "temperature": temperature,
    });
    if ep.backend == super::models::Backend::OpenRouter && ep.model == super::models::AUTO_ID {
        body["model"] = json!(super::models::AUTO_MODELS[0]);
        body["models"] = json!(super::models::AUTO_MODELS);
    }
    let mut req = agent_for(&ep.base)
        .post(&format!("{}/chat/completions", ep.base))
        .set("Content-Type", "application/json")
        .set("HTTP-Referer", "https://github.com/burkiuze/Forge")
        .set("X-Title", "Forge");
    if !ep.key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", ep.key.trim()));
    }
    let resp = match req.send_json(body) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                .unwrap_or(text);
            return Err(format!("HTTP {code}: {msg}"));
        }
        Err(e) => return Err(format!("Connection failed: {e}")),
    };

    let mut text = String::new();
    let mut model = ep.model.clone();
    let reader = BufReader::new(resp.into_reader());
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let line = line.map_err(|e| format!("Stream error: {e}"))?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if let Some(err) = v.get("error") {
            return Err(err["message"]
                .as_str()
                .unwrap_or("provider error")
                .to_string());
        }
        if let Some(m) = v["model"].as_str() {
            model = m.to_string();
        }
        if let Some(d) = v["choices"][0]["delta"]["content"].as_str() {
            if !d.is_empty() {
                text.push_str(d);
                on_delta(d);
            }
        }
    }
    if text.trim().is_empty() {
        return Err(
            "Model returned an empty reply (it may be rate-limited — try another model).".into(),
        );
    }
    Ok(Reply { text, model })
}

/// Quick key check against OpenRouter.
pub fn check_openrouter_key(key: &str) -> Result<String, String> {
    let r = agent()
        .get(&format!("{}/key", crate::config::OPENROUTER_BASE))
        .set("Authorization", &format!("Bearer {}", key.trim()))
        .call();
    match r {
        Ok(r) => {
            let v: Value = r.into_json().map_err(|e| e.to_string())?;
            let label = v["data"]["label"].as_str().unwrap_or("key");
            let free = v["data"]["is_free_tier"].as_bool().unwrap_or(false);
            Ok(format!(
                "OK · {label}{}",
                if free { " · free tier" } else { "" }
            ))
        }
        Err(ureq::Error::Status(c, _)) => Err(format!("Rejected (HTTP {c})")),
        Err(e) => Err(e.to_string()),
    }
}
