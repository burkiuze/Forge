//! Agent memory.
//!
//! * **TencentDB Agent Memory** (<https://github.com/TencentCloud/TencentDB-Agent-Memory>):
//!   every finished turn is written to Memory Core as L0 conversation (`/v3/conversation/add`),
//!   which its pipeline distills into L1 atoms / L2 scenarios / L3 persona. Before each run Forge
//!   recalls with `/v3/atomic/search` (facts, preferences) and falls back to `/v3/conversation/search`.
//! * **Built-in**: an offline, file-based store with the same layering idea (L0 turns + L1 facts)
//!   and BM25 retrieval, used when no Memory Core is running.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum MemoryBackend {
    Off,
    #[default]
    Builtin,
    TencentDb,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub backend: MemoryBackend,
    /// Memory Core base URL (default port 8420).
    pub url: String,
    /// `KERNEL_AUTH_TOKEN` (optional when Memory Core has no apiKey).
    pub token: String,
    pub service_id: String,
    pub team_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub recall_limit: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: MemoryBackend::Builtin,
            url: "http://127.0.0.1:8420".into(),
            token: String::new(),
            service_id: "default".into(),
            team_id: "default".into(),
            user_id: whoami(),
            agent_id: "forge".into(),
            recall_limit: 6,
        }
    }
}

fn whoami() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "local".into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryItem {
    pub kind: String,
    pub text: String,
    pub score: f32,
}

#[derive(Serialize, Deserialize, Clone)]
struct Record {
    /// "turn" (L0) or "fact" (L1).
    kind: String,
    text: String,
    session: String,
    ts: u64,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub struct Memory {
    pub cfg: MemoryConfig,
}

impl Memory {
    pub fn new(cfg: MemoryConfig) -> Self {
        Self { cfg }
    }

    fn file() -> PathBuf {
        let d = crate::config::data_dir().join("memory");
        let _ = std::fs::create_dir_all(&d);
        d.join("forge-memory.jsonl")
    }

    // ---------------------------------------------------------------- TencentDB Agent Memory

    fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        let mut req = crate::ai::client::agent()
            .post(&format!("{}{}", self.cfg.url.trim_end_matches('/'), path))
            .set("Content-Type", "application/json")
            .set("x-tdai-service-id", &self.cfg.service_id)
            .set("x-tdai-team-id", &self.cfg.team_id)
            .set("x-tdai-user-id", &self.cfg.user_id)
            .set("x-tdai-agent-id", &self.cfg.agent_id);
        if !self.cfg.token.is_empty() {
            req = req.set("Authorization", &format!("Bearer {}", self.cfg.token));
        }
        let v: Value = match req.send_json(body) {
            Ok(r) => r.into_json().map_err(|e| e.to_string())?,
            Err(ureq::Error::Status(c, r)) => {
                return Err(format!("HTTP {c}: {}", r.into_string().unwrap_or_default()))
            }
            Err(e) => return Err(format!("Memory Core unreachable: {e}")),
        };
        if v["code"].as_i64().unwrap_or(0) != 0 {
            return Err(v["message"].as_str().unwrap_or("memory error").to_string());
        }
        Ok(v["data"].clone())
    }

    pub fn health(&self) -> Result<String, String> {
        match self.cfg.backend {
            MemoryBackend::Off => Ok("off".into()),
            MemoryBackend::Builtin => Ok(format!("built-in · {} records", self.load().len())),
            MemoryBackend::TencentDb => {
                let r = crate::ai::client::agent()
                    .get(&format!("{}/health", self.cfg.url.trim_end_matches('/')))
                    .call()
                    .map_err(|e| e.to_string())?;
                let v: Value = r.into_json().unwrap_or_default();
                Ok(format!(
                    "TencentDB Agent Memory {} · {}",
                    v["version"].as_str().unwrap_or(""),
                    v["status"].as_str().unwrap_or("ok")
                ))
            }
        }
    }

    /// Retrieve memories relevant to `query`.
    pub fn recall(&self, query: &str) -> Vec<MemoryItem> {
        let limit = self.cfg.recall_limit.max(1);
        match self.cfg.backend {
            MemoryBackend::Off => vec![],
            MemoryBackend::Builtin => self.local_search(query, limit),
            MemoryBackend::TencentDb => {
                let q: String = query.chars().take(2000).collect();
                let mut out = vec![];
                if let Ok(d) = self.post("/v3/atomic/search", json!({"query": q, "limit": limit})) {
                    for it in d["items"].as_array().cloned().unwrap_or_default() {
                        out.push(MemoryItem {
                            kind: it["type"].as_str().unwrap_or("atom").to_string(),
                            text: it["content"].as_str().unwrap_or_default().to_string(),
                            score: it["score"].as_f64().unwrap_or(0.0) as f32,
                        });
                    }
                }
                if out.len() < limit {
                    if let Ok(d) = self.post(
                        "/v3/conversation/search",
                        json!({"query": q, "limit": limit - out.len()}),
                    ) {
                        for it in d["messages"].as_array().cloned().unwrap_or_default() {
                            out.push(MemoryItem {
                                kind: format!("L0 {}", it["role"].as_str().unwrap_or("")),
                                text: it["content"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .chars()
                                    .take(600)
                                    .collect(),
                                score: it["score"].as_f64().unwrap_or(0.0) as f32,
                            });
                        }
                    }
                }
                out
            }
        }
    }

    /// Persist a finished turn (user prompt + final agent answer).
    pub fn store_turn(&self, session: &str, user: &str, assistant: &str) -> Result<(), String> {
        match self.cfg.backend {
            MemoryBackend::Off => Ok(()),
            MemoryBackend::Builtin => {
                self.append(Record {
                    kind: "turn".into(),
                    text: format!("User: {user}\nForge: {assistant}"),
                    session: session.into(),
                    ts: now(),
                })?;
                for fact in extract_facts(user) {
                    self.append(Record {
                        kind: "fact".into(),
                        text: fact,
                        session: session.into(),
                        ts: now(),
                    })?;
                }
                Ok(())
            }
            MemoryBackend::TencentDb => {
                let clip = |s: &str| s.chars().take(8000).collect::<String>();
                let mut messages = vec![json!({"role": "user", "content": clip(user)})];
                if !assistant.trim().is_empty() {
                    messages.push(json!({"role": "assistant", "content": clip(assistant)}));
                }
                self.post(
                    "/v3/conversation/add",
                    json!({"session_id": session, "messages": messages}),
                )
                .map(|_| ())
            }
        }
    }

    /// Explicit "remember this" from an agent tool.
    pub fn remember(&self, session: &str, fact: &str) -> Result<(), String> {
        match self.cfg.backend {
            MemoryBackend::Off => Err("memory is off".into()),
            MemoryBackend::Builtin => self.append(Record { kind: "fact".into(), text: fact.into(), session: session.into(), ts: now() }),
            MemoryBackend::TencentDb => self
                .post("/v3/conversation/add", json!({"session_id": session, "messages": [{"role": "user", "content": format!("Please remember: {fact}")}]}))
                .map(|_| ()),
        }
    }

    pub fn clear_local(&self) {
        let _ = std::fs::remove_file(Self::file());
    }

    // ---------------------------------------------------------------- built-in store

    fn append(&self, r: Record) -> Result<(), String> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(Self::file())
            .map_err(|e| e.to_string())?;
        writeln!(f, "{}", serde_json::to_string(&r).unwrap()).map_err(|e| e.to_string())
    }

    fn load(&self) -> Vec<Record> {
        let Ok(f) = std::fs::File::open(Self::file()) else {
            return vec![];
        };
        std::io::BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect()
    }

    fn local_search(&self, query: &str, limit: usize) -> Vec<MemoryItem> {
        let recs = self.load();
        bm25(
            &recs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            query,
        )
        .into_iter()
        .take(limit)
        .map(|(i, score)| MemoryItem {
            kind: if recs[i].kind == "fact" {
                "L1 fact".into()
            } else {
                "L0 turn".into()
            },
            text: recs[i].text.clone(),
            score,
        })
        .collect()
    }
}

pub fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1)
        .map(str::to_string)
        .collect()
}

/// Okapi BM25 ranking → (doc index, score), best first, zero-score docs dropped.
pub fn bm25(docs: &[&str], query: &str) -> Vec<(usize, f32)> {
    let (k1, b) = (1.4f32, 0.75f32);
    let toks: Vec<Vec<String>> = docs.iter().map(|d| tokenize(d)).collect();
    let n = docs.len() as f32;
    if n == 0.0 {
        return vec![];
    }
    let avg = toks.iter().map(|t| t.len()).sum::<usize>() as f32 / n;
    let mut df: HashMap<&str, f32> = HashMap::new();
    for t in &toks {
        let mut seen: Vec<&str> = t.iter().map(String::as_str).collect();
        seen.sort();
        seen.dedup();
        for w in seen {
            *df.entry(w).or_default() += 1.0;
        }
    }
    let q = tokenize(query);
    let mut scored: Vec<(usize, f32)> = toks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let len = t.len() as f32;
            let s: f32 = q
                .iter()
                .map(|w| {
                    let tf = t.iter().filter(|x| *x == w).count() as f32;
                    if tf == 0.0 {
                        return 0.0;
                    }
                    let d = df.get(w.as_str()).copied().unwrap_or(0.0);
                    let idf = ((n - d + 0.5) / (d + 0.5) + 1.0).ln();
                    idf * tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * len / avg.max(1.0)))
                })
                .sum();
            (i, s)
        })
        .filter(|(_, s)| *s > 0.0)
        .collect();
    // Recent records win ties.
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.cmp(&a.0)));
    scored
}

/// L1 extraction heuristic: sentences that state preferences, rules or identity.
pub fn extract_facts(text: &str) -> Vec<String> {
    let cues = [
        "always",
        "never",
        "prefer",
        "i like",
        "i want",
        "my name",
        "i am",
        "remember",
        "don't",
        "do not",
        "use ",
        "hep",
        "asla",
        "tercih",
        "seviyorum",
        "istiyorum",
        "benim",
        "adım",
        "hatırla",
        "kullan",
        "olsun",
        "yapma",
    ];
    text.split(['.', '!', '\n', '?'])
        .map(str::trim)
        .filter(|s| s.chars().count() > 8 && s.chars().count() < 300)
        .filter(|s| {
            let l = s.to_lowercase();
            cues.iter().any(|c| l.contains(c))
        })
        .map(str::to_string)
        .collect()
}
