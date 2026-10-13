//! Multi-agent orchestrator.
//!
//! ```text
//!  user ─► Router ─┬─ chat ──────────────► Assistant ─► answer
//!                  └─ build ─► Planner ─► Builder ⇄ tools (scene)
//!                                           │  └─ ask_coder ─► Coder ─► set_script
//!                                           ▼
//!                                        Reviewer ─► (fixes → Builder) ─► summary
//! ```
//! Agents run on a worker thread; scene tools execute on the UI thread through
//! `AgentEvent::Tool` + a reply channel, so the scene never needs locking.

use super::client::{stream_chat, Msg};
use super::models::{self, Endpoint};
use super::tools::{self, ToolCall, SCRIPT_DOCS, TOOL_DOCS};
use crate::config::Settings;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Router,
    Assistant,
    Planner,
    Builder,
    Coder,
    Reviewer,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Router => "Router",
            Role::Assistant => "Forge AI",
            Role::Planner => "Planner",
            Role::Builder => "Builder",
            Role::Coder => "Coder",
            Role::Reviewer => "Reviewer",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Role::Router => "🧭",
            Role::Assistant => "✨",
            Role::Planner => "🗺",
            Role::Builder => "🔨",
            Role::Coder => "⌨",
            Role::Reviewer => "🔍",
        }
    }
}

pub enum AgentEvent {
    Start { role: Role, model: String },
    Delta { text: String },
    End { model: String },
    Tool { call: ToolCall, reply: Sender<Result<String, String>> },
    Info(String),
    Error(String),
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Agents may edit the scene.
    Agent,
    /// Plain Q&A, no tools.
    Ask,
}

pub struct Job {
    pub prompt: String,
    pub mode: Mode,
    pub history: Vec<Msg>,
    pub scene_json: String,
    pub selected: Option<String>,
    pub settings: Settings,
    pub openrouter_key: String,
}

pub struct AgentHandle {
    pub rx: Receiver<AgentEvent>,
    pub cancel: Arc<AtomicBool>,
}

pub fn spawn(job: Job, ctx: egui::Context) -> AgentHandle {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    std::thread::Builder::new()
        .name("forge-agent".into())
        .spawn(move || {
            let mut o = Orchestrator { job, tx: tx.clone(), cancel: c2, ctx: ctx.clone() };
            if let Err(e) = o.run() {
                let _ = tx.send(AgentEvent::Error(e));
            }
            let _ = tx.send(AgentEvent::Done);
            ctx.request_repaint();
        })
        .expect("spawn agent thread");
    AgentHandle { rx, cancel }
}

struct Orchestrator {
    job: Job,
    tx: Sender<AgentEvent>,
    cancel: Arc<AtomicBool>,
    ctx: egui::Context,
}

const PERSONA: &str = "You are Forge AI, the built-in assistant of the Forge game engine (an AI-native alternative to Unity, written in Rust). \
Always answer in the same language the user writes in (e.g. Turkish → Turkish). Be concise and practical.";

impl Orchestrator {
    fn send(&self, e: AgentEvent) {
        let _ = self.tx.send(e);
        self.ctx.request_repaint();
    }

    fn model_for(&self, role: Role) -> String {
        let s = &self.job.settings;
        let o = match role {
            Role::Planner => &s.planner_model,
            Role::Builder => &s.builder_model,
            Role::Reviewer => &s.reviewer_model,
            Role::Coder => &s.coder_model,
            Role::Router => &s.router_model,
            _ => &s.selected_model,
        };
        if o.trim().is_empty() { s.selected_model.clone() } else { o.clone() }
    }

    fn endpoint(&self, role: Role) -> Endpoint {
        models::resolve(&self.model_for(role), &self.job.settings, &self.job.openrouter_key)
    }

    /// One streamed LLM turn for `role`. Falls back to the Auto router once if the chosen model fails.
    fn call(&self, role: Role, messages: &[Msg]) -> Result<String, String> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let ep = self.endpoint(role);
        self.send(AgentEvent::Start { role, model: models::display_name(&ep.model) });
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let emit = move |d: &str| {
            let _ = tx.send(AgentEvent::Delta { text: d.to_string() });
            ctx.request_repaint();
        };
        let t = self.job.settings.temperature;
        match stream_chat(&ep, messages, t, &self.cancel, emit.clone()) {
            Ok(r) => {
                self.send(AgentEvent::End { model: r.model });
                Ok(r.text)
            }
            Err(e) if e != "Cancelled" && ep.model != models::AUTO_ID && ep.backend == models::Backend::OpenRouter => {
                self.send(AgentEvent::Info(format!("{} failed ({e}) — retrying with Auto", models::display_name(&ep.model))));
                let mut fb = ep.clone();
                fb.model = models::AUTO_ID.into();
                let r = stream_chat(&fb, messages, t, &self.cancel, emit)?;
                self.send(AgentEvent::End { model: r.model });
                Ok(r.text)
            }
            Err(e) => {
                self.send(AgentEvent::End { model: String::new() });
                Err(e)
            }
        }
    }

    /// Run a tool on the UI thread and wait for its result.
    fn tool(&self, call: ToolCall) -> Result<String, String> {
        let (rtx, rrx) = channel();
        self.send(AgentEvent::Tool { call, reply: rtx });
        rrx.recv().map_err(|_| "editor closed".to_string())?
    }

    fn context_block(&self) -> String {
        let mut s = format!("Current scene: {}", self.job.scene_json);
        if let Some(sel) = &self.job.selected {
            s.push_str(&format!("\nSelected entity: {sel}"));
        }
        s
    }

    fn run(&mut self) -> Result<(), String> {
        let wants_build = self.job.mode == Mode::Agent && self.route()?;
        if !wants_build {
            let mut msgs = vec![Msg::system(format!(
                "{PERSONA}\nYou can explain Forge features, game design, and Rhai scripting.\n{SCRIPT_DOCS}\n{}",
                self.context_block()
            ))];
            msgs.extend(self.job.history.iter().cloned());
            msgs.push(Msg::user(&self.job.prompt));
            self.call(Role::Assistant, &msgs)?;
            return Ok(());
        }

        // 1. Plan
        let plan = if self.job.settings.use_planner {
            let msgs = vec![
                Msg::system(format!(
                    "{PERSONA}\nYou are the PLANNER agent. Turn the user's request into a short numbered build plan (3-8 steps) \
                     for the Builder agent who edits the scene with tools (entities, transforms, colors, rigidbodies, cameras, Rhai scripts). \
                     Include concrete positions/sizes/colors where useful and which entities need scripts. Do not write code or tool calls.\n{}",
                    self.context_block()
                )),
                Msg::user(&self.job.prompt),
            ];
            Some(self.call(Role::Planner, &msgs)?)
        } else {
            None
        };

        // 2. Build (+ optional review round)
        let mut feedback: Option<String> = None;
        let rounds = if self.job.settings.use_reviewer { 2 } else { 1 };
        for round in 0..rounds {
            let summary = self.build(plan.as_deref(), feedback.as_deref())?;
            if !self.job.settings.use_reviewer || round + 1 == rounds {
                let _ = summary;
                break;
            }
            // 3. Review
            let scene_now = self.tool(ToolCall { name: "get_scene".into(), args: serde_json::json!({}) })?;
            let msgs = vec![
                Msg::system(format!(
                    "{PERSONA}\nYou are the REVIEWER agent. Check whether the scene satisfies the user's request. \
                     If it does, reply with the single word APPROVED followed by a one-paragraph summary for the user. \
                     Otherwise list the concrete fixes needed (max 5 bullets)."
                )),
                Msg::user(format!("Request: {}\n\nScene now: {scene_now}", self.job.prompt)),
            ];
            let verdict = self.call(Role::Reviewer, &msgs)?;
            if verdict.trim_start().to_ascii_uppercase().starts_with("APPROVED") {
                break;
            }
            feedback = Some(verdict);
        }
        Ok(())
    }

    /// Router: cheap heuristic, or an LLM decision when a router model is configured (e.g. Mercury Decide).
    fn route(&self) -> Result<bool, String> {
        let p = self.job.prompt.to_lowercase();
        let build_words = [
            "create", "make", "build", "add", "spawn", "place", "move", "delete", "remove", "change", "set", "color", "script",
            "level", "game", "scene", "player", "enemy", "rotate", "scale", "generate", "fix", "duplicate", "camera",
            "oluştur", "olustur", "yap", "ekle", "koy", "taşı", "tasi", "sil", "değiştir", "degistir", "renk", "boya", "seviye",
            "oyun", "sahne", "oyuncu", "düşman", "dusman", "döndür", "büyüt", "küçült", "kur", "yerleştir", "kamera", "zıpla",
            "hareket", "düzelt", "kopyala", "platform", "harita", "bölüm",
        ];
        let question = p.trim_end().ends_with('?')
            && !build_words.iter().take(12).any(|w| p.starts_with(w));
        let heuristic = build_words.iter().any(|w| p.contains(w)) && !(question && p.len() < 60);
        if self.job.settings.router_model.trim().is_empty() {
            return Ok(heuristic);
        }
        let msgs = vec![
            Msg::system(
                "Classify the user's message for a game-engine assistant. Answer with exactly one word: \
                 BUILD if it asks to create/modify/delete anything in the scene or game (objects, levels, scripts, cameras), \
                 CHAT if it is a question or conversation.",
            ),
            Msg::user(&self.job.prompt),
        ];
        match self.call(Role::Router, &msgs) {
            Ok(r) => Ok(r.to_ascii_uppercase().contains("BUILD")),
            Err(_) => Ok(heuristic),
        }
    }

    fn build(&self, plan: Option<&str>, feedback: Option<&str>) -> Result<String, String> {
        let mut msgs = vec![Msg::system(format!(
            "{PERSONA}\nYou are the BUILDER agent. You edit the open scene ONLY through tools.\n{TOOL_DOCS}\n{SCRIPT_DOCS}\n\
             Work efficiently: batch with create_many, give every entity a clear unique name, keep things on the ground, \
             add a Main Camera that frames the action (follow the player for games). For gameplay logic you may write scripts \
             yourself with set_script or delegate with ask_coder.\n{}",
            self.context_block()
        ))];
        msgs.extend(self.job.history.iter().cloned());
        let mut task = format!("User request: {}", self.job.prompt);
        if let Some(p) = plan {
            task.push_str(&format!("\n\nPlan from the Planner:\n{p}"));
        }
        if let Some(f) = feedback {
            task.push_str(&format!("\n\nReviewer feedback — fix these now:\n{f}"));
        }
        msgs.push(Msg::user(task));

        let max = self.job.settings.max_agent_steps.max(1);
        for step in 0..max {
            let reply = self.call(Role::Builder, &msgs)?;
            let calls = tools::parse_tool_calls(&reply);
            if calls.is_empty() {
                return Ok(reply);
            }
            let mut results = String::new();
            for call in calls {
                if self.cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                let r = if call.name == "ask_coder" {
                    self.coder(&call)
                } else {
                    self.tool(call.clone())
                };
                let line = match r {
                    Ok(s) => format!("✔ {}: {}", call.name, truncate(&s, 1800)),
                    Err(e) => format!("✘ {}: ERROR {e}", call.name),
                };
                results.push_str(&line);
                results.push('\n');
            }
            msgs.push(Msg::assistant(reply));
            let nudge = if step + 2 >= max { "\nYou are almost out of steps: finish now and summarize." } else { "" };
            msgs.push(Msg::user(format!("Tool results:\n{results}{nudge}")));
        }
        Ok(String::new())
    }

    /// Coder sub-agent: writes a Rhai script for one entity, validates it, retries once on compile errors.
    fn coder(&self, call: &ToolCall) -> Result<String, String> {
        let target = call.args.get("target").and_then(|t| t.as_str()).unwrap_or_default().to_string();
        let goal = call.args.get("goal").and_then(|t| t.as_str()).unwrap_or_default().to_string();
        let mut msgs = vec![
            Msg::system(format!(
                "You are the CODER agent of the Forge engine. Write a Rhai script for one entity.\n{SCRIPT_DOCS}\n\
                 Reply with ONLY the code inside one ```rhai block."
            )),
            Msg::user(format!("Entity: {target}\nGoal: {goal}\nScene: {}", self.job.scene_json)),
        ];
        for _ in 0..2 {
            let reply = self.call(Role::Coder, &msgs)?;
            let code = extract_code(&reply);
            match rhai::Engine::new().compile(&code) {
                Ok(_) => {
                    return self.tool(ToolCall {
                        name: "set_script".into(),
                        args: serde_json::json!({"target": target, "code": code}),
                    });
                }
                Err(e) => {
                    msgs.push(Msg::assistant(reply));
                    msgs.push(Msg::user(format!("Compile error: {e}. Fix it and reply with the full corrected script.")));
                }
            }
        }
        Err("coder could not produce a compiling script".into())
    }
}

fn extract_code(reply: &str) -> String {
    if let Some(start) = reply.find("```") {
        let after = &reply[start + 3..];
        let after = after.split_once('\n').map(|(_, b)| b).unwrap_or(after);
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    reply.trim().to_string()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}
