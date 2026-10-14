//! Multi-agent orchestrator.
//!
//! ```text
//!               ┌──────── Agent Memory (TencentDB Agent Memory / built-in) ───────┐
//!               │ recall before the run                     store after the run │
//!  user ─► Router ─┬─ Ask ───────► Assistant ⇄ web_search / fetch_url / memory ─► answer
//!                  ├─ Fast ──────────────────► Builder ⇄ scene + web + memory tools
//!                  └─ Planning ─► Planner ─► Builder ⇄ tools ─► (ask_coder ─► Coder)
//!                                                  └──► Reviewer ─► fixes ─► Builder
//! ```
//! Agents run on a worker thread; scene tools execute on the UI thread through
//! `AgentEvent::Tool` + a reply channel, so the scene never needs locking. Web, memory and
//! coder tools run directly on the worker thread.

use super::client::{stream_chat, Msg};
use super::memory::{Memory, MemoryConfig};
use super::models::{self, Endpoint};
use super::tools::{self, ToolCall, SCRIPT_DOCS, TOOL_DOCS};
use super::web::{self, SearchHit};
use crate::config::Settings;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Explore,
    Assistant,
    Planner,
    Builder,
    Coder,
    Reviewer,
}

impl Role {
    pub fn icon(self) -> &'static str {
        match self {
            Role::Explore => "🔭",
            Role::Assistant => "✨",
            Role::Planner => "🗺",
            Role::Builder => "🔨",
            Role::Coder => "⌨",
            Role::Reviewer => "🔍",
        }
    }
}

pub enum AgentEvent {
    /// Ask the user before a worker-side change (e.g. downloading an asset).
    Approve {
        call: ToolCall,
        reply: Sender<bool>,
    },
    Start {
        role: Role,
        model: String,
    },
    Delta {
        text: String,
    },
    End {
        model: String,
    },
    Tool {
        call: ToolCall,
        reply: Sender<Result<String, String>>,
    },
    /// A tool that ran on the worker (web, memory, coder) — for display only.
    ToolDone {
        call: ToolCall,
        result: Result<String, String>,
    },
    Sources(Vec<SearchHit>),
    Todos(Vec<(String, String)>),
    Compacted(String),
    Memories(Vec<String>),
    Info(String),
    Error(String),
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize, Default)]
pub enum Mode {
    /// Planner → Builder (→ Reviewer): best for bigger tasks.
    #[default]
    Planning,
    /// Builder only: quick edits.
    Fast,
    /// Q&A — no scene edits (web + memory tools still available).
    Ask,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Planning => crate::i18n::t("Planning", "Planlama"),
            Mode::Fast => crate::i18n::t("Fast", "Hızlı"),
            Mode::Ask => crate::i18n::t("Ask", "Sor"),
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Mode::Planning => crate::i18n::t(
                "Agent plans before building. Use for bigger tasks.",
                "Ajan önce plan yapar. Büyük işler için.",
            ),
            Mode::Fast => crate::i18n::t(
                "Agent edits the scene directly. Use for quick changes.",
                "Ajan sahneyi doğrudan düzenler. Hızlı değişiklikler için.",
            ),
            Mode::Ask => crate::i18n::t(
                "Answers questions; never edits the scene.",
                "Soruları yanıtlar; sahneyi değiştirmez.",
            ),
        }
    }
}

pub struct Job {
    pub prompt: String,
    pub mode: Mode,
    pub history: Vec<Msg>,
    pub scene_json: String,
    pub selected: Option<String>,
    pub settings: Settings,
    pub openrouter_key: String,
    pub web: bool,
    pub memory: MemoryConfig,
    pub session_id: String,
    /// Summary of older turns (opencode-style compaction).
    pub summary: String,
    /// Project folder (asset imports land here).
    pub project_root: std::path::PathBuf,
    pub permission: crate::config::Permission,
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
            let memory = Memory::new(job.memory.clone());
            let mut o = Orchestrator {
                job,
                tx: tx.clone(),
                cancel: c2,
                ctx: ctx.clone(),
                memory,
                memories: String::new(),
                last_answer: String::new(),
            };
            if let Err(e) = o.run() {
                let _ = tx.send(AgentEvent::Error(e));
            }
            o.persist();
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
    memory: Memory,
    memories: String,
    last_answer: String,
}

const PERSONA: &str = "You are Forge AI, the built-in agent of the Forge game engine (an open-source, AI-native alternative to Unity / Unreal, written in Rust). \
Always answer in the same language the user writes in (e.g. Turkish → Turkish). Be concise and practical.";

const ASK_TOOLS: &str = r##"## Tools
You may call tools with fenced blocks (one JSON object per block), then STOP and wait for results:
```tool
{"tool": "web_search", "args": {"query": "rust ecs tutorial"}}
```
- web_search {query} → web results searched from the user's device. Use it for current facts, docs, assets, tutorials.
- fetch_url {url} → readable text of a page (read the best result before answering).
- memory_search {query} → the user's long-term memories.
- remember {fact} → save a durable preference/fact.
- get_scene {} · list_files {path} · read_file {path} · grep {pattern} → inspect the scene and project files (read-only).
- task {agent: "explore", prompt} → delegate a research sub-task to the Explore subagent.
When you have enough information, answer WITHOUT tool blocks and cite sources as markdown links.
"##;

/// Tools an agent may use without changing anything (Ask mode / Explore subagent).
pub const READ_ONLY_TOOLS: &[&str] = &[
    "web_search",
    "fetch_url",
    "memory_search",
    "remember",
    "get_scene",
    "list_files",
    "read_file",
    "grep",
    "task",
    "todo_write",
    "asset_search",
    "list_assets",
    "inspect_asset",
];

/// How the Builder should work — assets first, then logic, then polish.
const BUILDER_GUIDE: &str = "## How to work
You can build complete games from an empty scene, extend them, or fix one specific part the user points at.
1. Understand: call get_scene and list_assets first. For a fix, change only what the user asked about (inspect with get_scene / grep / read_file) and keep everything else intact.
2. Plan with todo_write for anything bigger than a single change, and keep it updated.
3. ASSETS FIRST — never model characters, props or vehicles out of primitive shapes:
   - Reuse what is in the project (list_assets, inspect_asset to learn a model's size).
   - Otherwise find real assets: asset_search {source:\"polyhaven\", kind:\"models\"} for 3D models, kind:\"textures\" or source:\"ambientcg\" for surfaces, then asset_import and place models with create_entity {mesh:\"model\", model: path} (real size is applied automatically).
   - If no suitable model exists (e.g. a specific character), use the closest real asset, a simple stand-in object clearly named \"<Thing> (placeholder)\", and tell the user they can import their own model (Content Browser › Import from device, glTF/GLB) — never build a figure out of spheres and cylinders.
   - Primitives are only for level geometry: ground, walls, platforms, ramps, triggers, pickups.
4. Gameplay: give the player a rigidbody + controller script (set_script or ask_coder), add goals (collectibles, enemies, timers), a HUD via hud(), win/lose conditions, and a Main Camera that follows the player.
5. Feel: generate_sound for coins/jumps/hits and trigger them with play_sound in scripts; animator presets (bob, spin, pulse, hover, door…) or create_animation for life; materials, textures and lights for looks.
6. Batch with create_many, give every entity a clear unique name, keep things on the ground (y = height/2).
7. Finish with a short summary of what you built and how to play (keys), plus anything the user should import or decide.";

const EXPLORE_PROMPT: &str = "You are the EXPLORE subagent of the Forge engine: a search specialist. Your role is EXCLUSIVELY to search and analyze \
(scene, project files, the web, memory). You MUST NOT modify anything. Return a concise report with the facts, names and links the caller needs.";

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
            Role::Explore => &s.router_model,
            _ => &s.selected_model,
        };
        if o.trim().is_empty() {
            s.selected_model.clone()
        } else {
            o.clone()
        }
    }

    fn endpoint(&self, role: Role) -> Endpoint {
        models::resolve(
            &self.model_for(role),
            &self.job.settings,
            &self.job.openrouter_key,
        )
    }

    /// One streamed LLM turn for `role`. Falls back to Auto once if the chosen cloud model fails.
    fn call(&mut self, role: Role, messages: &[Msg]) -> Result<String, String> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let ep = self.endpoint(role);
        self.send(AgentEvent::Start {
            role,
            model: models::display_name(&ep.model),
        });
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let emit = move |d: &str| {
            let _ = tx.send(AgentEvent::Delta {
                text: d.to_string(),
            });
            ctx.request_repaint();
        };
        let t = self.job.settings.temperature;
        let r = match stream_chat(&ep, messages, t, &self.cancel, emit.clone()) {
            Ok(r) => Ok(r),
            Err(e)
                if e != "Cancelled"
                    && ep.model != models::AUTO_ID
                    && ep.backend == models::Backend::OpenRouter =>
            {
                self.send(AgentEvent::Info(format!(
                    "{} -> Auto ({e})",
                    models::display_name(&ep.model)
                )));
                let mut fb = ep.clone();
                fb.model = models::AUTO_ID.into();
                stream_chat(&fb, messages, t, &self.cancel, emit)
            }
            Err(e) => Err(e),
        };
        match r {
            Ok(r) => {
                self.send(AgentEvent::End { model: r.model });
                self.last_answer = tools::strip_tool_blocks(&r.text);
                Ok(r.text)
            }
            Err(e) => {
                self.send(AgentEvent::End {
                    model: String::new(),
                });
                Err(e)
            }
        }
    }

    /// Manual permission mode: block until the user accepts or rejects.
    fn approve(&self, call: &ToolCall) -> bool {
        if self.job.permission == crate::config::Permission::Auto {
            return true;
        }
        let (tx, rx) = channel();
        self.send(AgentEvent::Approve {
            call: call.clone(),
            reply: tx,
        });
        rx.recv().unwrap_or(false)
    }

    /// Run a scene tool on the UI thread and wait for its result.
    fn ui_tool(&self, call: ToolCall) -> Result<String, String> {
        let (rtx, rrx) = channel();
        self.send(AgentEvent::Tool { call, reply: rtx });
        rrx.recv().map_err(|_| "editor closed".to_string())?
    }

    /// Dispatch any tool: worker-side tools here, scene tools on the UI thread.
    fn tool(&mut self, call: &ToolCall) -> Result<String, String> {
        let worker = matches!(
            call.name.as_str(),
            "web_search"
                | "fetch_url"
                | "memory_search"
                | "remember"
                | "ask_coder"
                | "task"
                | "todo_write"
        );
        if !worker {
            return self.ui_tool(call.clone());
        }
        let arg = |k: &str| {
            call.args
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let r = match call.name.as_str() {
            "web_search" if !self.job.web => {
                Err("web access is disabled in the Agent panel (🌐)".into())
            }
            "fetch_url" if !self.job.web => {
                Err("web access is disabled in the Agent panel (🌐)".into())
            }
            "web_search" => web::search(&arg("query"), 6).map(|hits| {
                self.send(AgentEvent::Sources(hits.clone()));
                serde_json::to_string_pretty(&hits).unwrap_or_default()
            }),
            "fetch_url" => web::fetch_text(&arg("url"), 6000),
            "memory_search" => {
                let items = self.memory.recall(&arg("query"));
                Ok(if items.is_empty() {
                    "no memories found".into()
                } else {
                    items
                        .iter()
                        .map(|m| format!("- [{}] {}", m.kind, m.text))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            }
            "remember" => self
                .memory
                .remember(&self.job.session_id, &arg("fact"))
                .map(|_| "stored".into()),
            "ask_coder" => self.coder(call),
            "task" => self.subagent(&arg("agent"), &arg("prompt")),
            "asset_search" if !self.job.web => {
                Err("web access is disabled in the Agent panel (🌐)".into())
            }
            "asset_search" => crate::store::search(
                crate::store::Source::parse(&arg("source")),
                crate::store::Kind::parse(&arg("kind")),
                &arg("query"),
                12,
            )
            .map(|items| {
                items
                    .iter()
                    .map(|i| {
                        format!(
                            "- id={} name=\"{}\" source={} kind={:?} tags={}",
                            i.id,
                            i.name,
                            i.source.label(),
                            i.kind,
                            i.tags.join(",")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            "asset_import" if !self.approve(call) => Err("The user rejected this action.".into()),
            "asset_import" => crate::store::import(
                crate::store::Source::parse(&arg("source")),
                crate::store::Kind::parse(&arg("kind")),
                &arg("id"),
                &self.job.project_root,
            )
            .map(|imp| serde_json::to_string(&imp).unwrap_or_default()),
            "todo_write" => {
                let todos: Vec<(String, String)> = call.args["todos"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|t| {
                                (
                                    t["content"]
                                        .as_str()
                                        .or(t["text"].as_str())
                                        .unwrap_or_default()
                                        .to_string(),
                                    t["status"].as_str().unwrap_or("pending").to_string(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let n = todos.len();
                self.send(AgentEvent::Todos(todos));
                Ok(format!("{n} todos tracked"))
            }
            _ => unreachable!(),
        };
        self.send(AgentEvent::ToolDone {
            call: call.clone(),
            result: r.clone(),
        });
        r
    }

    fn context_block(&self) -> String {
        let mut s = String::new();
        if self.job.mode != Mode::Ask || !self.job.scene_json.is_empty() {
            s.push_str(&format!("Current scene: {}", self.job.scene_json));
        }
        if let Some(sel) = &self.job.selected {
            s.push_str(&format!("\nSelected entity: {sel}"));
        }
        if !self.job.summary.is_empty() {
            s.push_str(&format!(
                "\n\nSummary of the earlier conversation:\n{}",
                self.job.summary
            ));
        }
        if !self.memories.is_empty() {
            s.push_str(&format!("\n\nRelevant long-term memories about this user/project (use them, don't repeat them verbatim):\n{}", self.memories));
        }
        if !self.job.web {
            s.push_str("\n\nWeb access is OFF: do not call web_search or fetch_url.");
        }
        s
    }

    fn recall(&mut self) {
        let items = self.memory.recall(&self.job.prompt);
        if !items.is_empty() {
            self.memories = items
                .iter()
                .map(|m| format!("- {}", m.text.replace('\n', " ")))
                .collect::<Vec<_>>()
                .join("\n");
            self.send(AgentEvent::Memories(
                items.into_iter().map(|m| m.text).collect(),
            ));
        }
    }

    fn persist(&self) {
        if !self.last_answer.trim().is_empty() || !self.job.prompt.is_empty() {
            if let Err(e) =
                self.memory
                    .store_turn(&self.job.session_id, &self.job.prompt, &self.last_answer)
            {
                let _ = self.tx.send(AgentEvent::Info(format!("memory: {e}")));
            }
        }
    }

    /// opencode-style compaction: fold old turns into a short summary once the history grows.
    fn compact(&mut self) {
        let chars: usize = self.job.history.iter().map(|m| m.content.len()).sum();
        if self.job.history.len() <= 8 && chars < 12_000 {
            return;
        }
        let keep = 4.min(self.job.history.len());
        let old: Vec<Msg> = self
            .job
            .history
            .drain(..self.job.history.len() - keep)
            .collect();
        let transcript: String = old
            .iter()
            .map(|m| {
                format!(
                    "{}: {}\n",
                    m.role,
                    m.content.chars().take(2000).collect::<String>()
                )
            })
            .collect();
        let msgs = vec![
            Msg::system("Summarize what was done in this conversation, like a pull request description. 3-6 sentences. Keep names of entities, files, decisions and open questions. Same language as the conversation."),
            Msg::user(format!("{}\n\n{transcript}", self.job.summary)),
        ];
        let saved = self.last_answer.clone();
        if let Ok(sum) = self.call(Role::Assistant, &msgs) {
            self.job.summary = sum.clone();
            self.send(AgentEvent::Compacted(sum));
        }
        self.last_answer = saved;
    }

    fn run(&mut self) -> Result<(), String> {
        self.recall();
        self.compact();
        if self.job.mode == Mode::Ask {
            return self.ask();
        }

        // 1. Plan
        let plan = if self.job.mode == Mode::Planning && self.job.settings.use_planner {
            let msgs = vec![
                Msg::system(format!(
                    "{PERSONA}\nYou are the PLANNER agent. Turn the user's request into a short numbered build plan (3-8 steps) \
                     for the Builder agent who edits the scene with tools (entities, transforms, materials, lights, rigidbodies, cameras, Rhai scripts, web research). \
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
        let review = self.job.mode == Mode::Planning && self.job.settings.use_reviewer;
        let rounds = if review { 2 } else { 1 };
        for round in 0..rounds {
            self.build(plan.as_deref(), feedback.as_deref())?;
            if !review || round + 1 == rounds {
                break;
            }
            let scene_now = self.ui_tool(ToolCall {
                name: "get_scene".into(),
                args: serde_json::json!({}),
            })?;
            let msgs = vec![
                Msg::system(format!(
                    "{PERSONA}\nYou are the REVIEWER agent. Check whether the scene satisfies the user's request. \
                     If it does, reply with the single word APPROVED followed by a one-paragraph summary for the user. \
                     Otherwise list the concrete fixes needed (max 5 bullets)."
                )),
                Msg::user(format!("Request: {}\n\nScene now: {scene_now}", self.job.prompt)),
            ];
            let verdict = self.call(Role::Reviewer, &msgs)?;
            if verdict
                .trim_start()
                .to_ascii_uppercase()
                .starts_with("APPROVED")
            {
                break;
            }
            feedback = Some(verdict);
        }
        Ok(())
    }

    /// Ask mode: a small tool loop with web + memory tools only.
    fn ask(&mut self) -> Result<(), String> {
        let mut msgs = vec![Msg::system(format!(
            "{PERSONA}\nYou can explain Forge features, game design, and Rhai scripting.\n{SCRIPT_DOCS}\n{ASK_TOOLS}\n{}",
            self.context_block()
        ))];
        msgs.extend(self.job.history.iter().cloned());
        msgs.push(Msg::user(&self.job.prompt));
        for _ in 0..5 {
            let reply = self.call(Role::Assistant, &msgs)?;
            let calls: Vec<ToolCall> = tools::parse_tool_calls(&reply)
                .into_iter()
                .filter(|c| READ_ONLY_TOOLS.contains(&c.name.as_str()))
                .collect();
            if calls.is_empty() {
                return Ok(());
            }
            let results = self.run_calls(&calls)?;
            msgs.push(Msg::assistant(reply));
            msgs.push(Msg::user(format!("Tool results:\n{results}")));
        }
        Ok(())
    }

    fn run_calls(&mut self, calls: &[ToolCall]) -> Result<String, String> {
        let mut results = String::new();
        for call in calls {
            if self.cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            let line = match self.tool(call) {
                Ok(s) => format!("✔ {}: {}", call.name, truncate(&s, 4000)),
                Err(e) => format!("✖ {}: ERROR {e}", call.name),
            };
            results.push_str(&line);
            results.push('\n');
        }
        Ok(results)
    }

    fn build(&mut self, plan: Option<&str>, feedback: Option<&str>) -> Result<String, String> {
        let mut msgs = vec![Msg::system(format!(
            "{PERSONA}\nYou are the BUILDER agent. You edit the open scene ONLY through tools.\n{TOOL_DOCS}\n{SCRIPT_DOCS}\n\
             {BUILDER_GUIDE}\n{}",
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
            let results = self.run_calls(&calls)?;
            msgs.push(Msg::assistant(reply));
            let nudge = if step + 2 >= max {
                "\nYou are almost out of steps: finish now and summarize."
            } else {
                ""
            };
            msgs.push(Msg::user(format!("Tool results:\n{results}{nudge}")));
        }
        Ok(String::new())
    }

    /// Generic subagent loop (opencode `task` tool): `explore` is read-only, `coder` may write scripts.
    fn subagent(&mut self, agent: &str, prompt: &str) -> Result<String, String> {
        let (role, system, allowed): (Role, String, Vec<&str>) = match agent {
            "coder" | "code" => (
                Role::Coder,
                format!("You are the CODER subagent of the Forge engine. Write and fix Rhai gameplay scripts.\n{SCRIPT_DOCS}\n{TOOL_DOCS}\nOnly use: get_scene, read_file, list_files, grep, write_file, set_script. Finish with a short report."),
                vec!["get_scene", "read_file", "list_files", "grep", "write_file", "set_script"],
            ),
            _ => (Role::Explore, format!("{EXPLORE_PROMPT}\n{ASK_TOOLS}"), READ_ONLY_TOOLS.iter().copied().filter(|t| *t != "task").collect()),
        };
        let saved = self.last_answer.clone();
        let mut msgs = vec![
            Msg::system(format!("{system}\nScene: {}", self.job.scene_json)),
            Msg::user(prompt),
        ];
        let mut report = String::new();
        for _ in 0..6 {
            let reply = self.call(role, &msgs)?;
            let calls: Vec<ToolCall> = tools::parse_tool_calls(&reply)
                .into_iter()
                .filter(|c| allowed.contains(&c.name.as_str()))
                .collect();
            if calls.is_empty() {
                report = tools::strip_tool_blocks(&reply);
                break;
            }
            let results = self.run_calls(&calls)?;
            msgs.push(Msg::assistant(reply));
            msgs.push(Msg::user(format!("Tool results:\n{results}")));
        }
        self.last_answer = saved;
        Ok(if report.is_empty() {
            "subagent finished without a report".into()
        } else {
            report
        })
    }

    /// Coder sub-agent: writes a Rhai script for one entity, validates it, retries once on compile errors.
    fn coder(&mut self, call: &ToolCall) -> Result<String, String> {
        let target = call
            .args
            .get("target")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();
        let goal = call
            .args
            .get("goal")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();
        let mut msgs = vec![
            Msg::system(format!(
                "You are the CODER agent of the Forge engine. Write a Rhai script for one entity.\n{SCRIPT_DOCS}\n\
                 Reply with ONLY the code inside one ```rhai block."
            )),
            Msg::user(format!("Entity: {target}\nGoal: {goal}\nScene: {}", self.job.scene_json)),
        ];
        let saved = self.last_answer.clone();
        for _ in 0..2 {
            let reply = self.call(Role::Coder, &msgs)?;
            let code = extract_code(&reply);
            match rhai::Engine::new().compile(&code) {
                Ok(_) => {
                    self.last_answer = saved;
                    return self.ui_tool(ToolCall {
                        name: "set_script".into(),
                        args: serde_json::json!({"target": target, "code": code}),
                    });
                }
                Err(e) => {
                    msgs.push(Msg::assistant(reply));
                    msgs.push(Msg::user(format!(
                        "Compile error: {e}. Fix it and reply with the full corrected script."
                    )));
                }
            }
        }
        self.last_answer = saved;
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
