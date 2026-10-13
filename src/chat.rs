//! Forge AI chat panel (right side).

use crate::ai::agent::{self, AgentEvent, AgentHandle, Job, Mode, Role};
use crate::ai::client::Msg;
use crate::ai::models::{self, CLOUD_MODELS};
use crate::ai::tools::{self, ToolCall};
use crate::theme;
use egui::{Color32, RichText};
use std::sync::atomic::Ordering;

pub enum Who {
    User,
    Agent(Role),
    Info,
    Error,
}

pub struct Entry {
    pub who: Who,
    pub model: String,
    pub text: String,
    pub tools: Vec<(ToolCall, Result<String, String>)>,
}

pub struct ChatPanel {
    pub entries: Vec<Entry>,
    pub input: String,
    pub mode: Mode,
    handle: Option<AgentHandle>,
    history: Vec<Msg>,
    pending_prompt: String,
}

impl Default for ChatPanel {
    fn default() -> Self {
        Self {
            entries: vec![Entry {
                who: Who::Agent(Role::Assistant),
                model: String::new(),
                text: "Merhaba! I'm Forge AI. Describe a game or a change and my agents will build it in the scene.\n\
                       Try: \"Bir platform oyunu yap: zıplayan oyuncu, 10 altın ve takip eden kamera\""
                    .into(),
                tools: vec![],
            }],
            input: String::new(),
            mode: Mode::Agent,
            handle: None,
            history: vec![],
            pending_prompt: String::new(),
        }
    }
}

/// Requests the chat panel makes of the editor.
pub enum ChatRequest {
    Send,
    SelectModel(String),
    Settings,
}

impl ChatPanel {
    pub fn busy(&self) -> bool {
        self.handle.is_some()
    }

    pub fn start(&mut self, job_fn: impl FnOnce(Vec<Msg>, Mode) -> Job, ctx: &egui::Context) {
        let prompt = std::mem::take(&mut self.input).trim().to_string();
        if prompt.is_empty() || self.busy() {
            return;
        }
        self.entries.push(Entry { who: Who::User, model: String::new(), text: prompt.clone(), tools: vec![] });
        self.pending_prompt = prompt;
        let hist = self.history.iter().rev().take(6).rev().cloned().collect();
        let job = job_fn(hist, self.mode);
        self.handle = Some(agent::spawn(job, ctx.clone()));
    }

    pub fn stop(&mut self) {
        if let Some(h) = &self.handle {
            h.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Drain agent events. Tool calls are executed by `exec` (on the UI thread).
    pub fn poll(&mut self, mut exec: impl FnMut(&ToolCall) -> Result<String, String>) {
        let Some(h) = &self.handle else { return };
        let mut done = false;
        while let Ok(ev) = h.rx.try_recv() {
            match ev {
                AgentEvent::Start { role, model } => {
                    self.entries.push(Entry { who: Who::Agent(role), model, text: String::new(), tools: vec![] });
                }
                AgentEvent::Delta { text } => {
                    if let Some(e) = self.entries.last_mut() {
                        e.text.push_str(&text);
                    }
                }
                AgentEvent::End { model } => {
                    if let Some(e) = self.entries.last_mut() {
                        if !model.is_empty() {
                            e.model = models::display_name(&model);
                        }
                    }
                }
                AgentEvent::Tool { call, reply } => {
                    let r = exec(&call);
                    let _ = reply.send(r.clone());
                    if let Some(e) = self.entries.iter_mut().rev().find(|e| matches!(e.who, Who::Agent(_))) {
                        e.tools.push((call, r));
                    }
                }
                AgentEvent::Info(t) => self.entries.push(Entry { who: Who::Info, model: String::new(), text: t, tools: vec![] }),
                AgentEvent::Error(t) => self.entries.push(Entry { who: Who::Error, model: String::new(), text: t, tools: vec![] }),
                AgentEvent::Done => done = true,
            }
        }
        if done {
            self.handle = None;
            // Keep a compact conversation memory: user prompt + last agent text.
            let last = self
                .entries
                .iter()
                .rev()
                .find(|e| matches!(e.who, Who::Agent(_)) && !e.text.is_empty())
                .map(|e| tools::strip_tool_blocks(&e.text))
                .unwrap_or_default();
            self.history.push(Msg::user(std::mem::take(&mut self.pending_prompt)));
            self.history.push(Msg::assistant(last));
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, current_model: &str, local: &[String]) -> Vec<ChatRequest> {
        let mut reqs = vec![];
        // Header
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
            theme::draw_logo(ui.painter(), r);
            ui.label(RichText::new("Forge AI").strong().size(15.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("⚙").on_hover_text("AI settings").clicked() {
                    reqs.push(ChatRequest::Settings);
                }
                if ui.small_button("🗑").on_hover_text("Clear chat").clicked() && !self.busy() {
                    *self = ChatPanel { mode: self.mode, ..Default::default() };
                }
            });
        });
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("model_pick")
                .selected_text(models::display_name(current_model))
                .width(ui.available_width() - 120.0)
                .show_ui(ui, |ui| {
                    ui.label(RichText::new("OpenRouter").small().weak());
                    for m in CLOUD_MODELS {
                        if ui.selectable_label(current_model == m.id, m.name).on_hover_text(m.blurb).clicked() {
                            reqs.push(ChatRequest::SelectModel(m.id.into()));
                        }
                    }
                    if !local.is_empty() {
                        ui.separator();
                        ui.label(RichText::new("Local").small().weak());
                        for m in local {
                            if ui.selectable_label(current_model == m, models::display_name(m)).clicked() {
                                reqs.push(ChatRequest::SelectModel(m.clone()));
                            }
                        }
                    }
                });
            ui.selectable_value(&mut self.mode, Mode::Agent, "Agent").on_hover_text("Agents can build & edit the scene");
            ui.selectable_value(&mut self.mode, Mode::Ask, "Ask").on_hover_text("Just chat, no scene edits");
        });
        ui.separator();

        // Input (bottom)
        egui::TopBottomPanel::bottom("chat_input").frame(egui::Frame::NONE).show_inside(ui, |ui| {
            ui.add_space(4.0);
            let busy = self.busy();
            let te = egui::TextEdit::multiline(&mut self.input)
                .hint_text("Ask Forge AI to build something…  (Enter = send, Shift+Enter = newline)")
                .desired_rows(3)
                .desired_width(f32::INFINITY);
            let r = ui.add(te);
            let enter = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
            ui.horizontal(|ui| {
                if busy {
                    ui.spinner();
                    ui.small("agents working…");
                    if ui.button("■ Stop").clicked() {
                        self.stop();
                    }
                } else {
                    ui.small(RichText::new(if self.mode == Mode::Agent { "Router › Planner › Builder ‹› tools" } else { "Q&A mode" }).weak());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let send = ui.add_enabled(!busy, egui::Button::new(RichText::new("Send ▶").color(Color32::WHITE)).fill(theme::ACCENT));
                    if (send.clicked() || enter) && !busy {
                        let text = self.input.trim_end_matches('\n').to_string();
                        self.input = text.clone();
                        reqs.push(ChatRequest::Send);
                    }
                });
            });
        });

        // Messages
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false; 2]).show(ui, |ui| {
            for e in &self.entries {
                message(ui, e);
                ui.add_space(6.0);
            }
        });
        reqs
    }
}

fn message(ui: &mut egui::Ui, e: &Entry) {
    let (title, color, fill) = match &e.who {
        Who::User => ("You".to_string(), theme::GOLD, theme::BG2),
        Who::Agent(r) => (format!("{} {}", r.icon(), r.label()), theme::ACCENT, theme::BG0),
        Who::Info => ("ℹ".to_string(), Color32::LIGHT_BLUE, theme::BG0),
        Who::Error => ("⚠ Error".to_string(), Color32::LIGHT_RED, theme::BG0),
    };
    egui::Frame::new().fill(fill).corner_radius(6).inner_margin(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).strong().color(color));
            if !e.model.is_empty() {
                ui.label(RichText::new(&e.model).small().weak());
            }
        });
        let text = tools::strip_tool_blocks(&e.text);
        if text.is_empty() && e.tools.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.small("thinking…");
            });
        }
        render_markdownish(ui, &text);
        for (call, res) in &e.tools {
            let (icon, c) = match res {
                Ok(_) => ("✔", Color32::from_rgb(120, 210, 140)),
                Err(_) => ("✘", Color32::LIGHT_RED),
            };
            egui::CollapsingHeader::new(RichText::new(format!("{icon} {}", call.name)).small().color(c))
                .id_salt((e as *const Entry as usize, call.name.as_str(), call.args.to_string().len()))
                .show(ui, |ui| {
                    ui.label(RichText::new(serde_json::to_string_pretty(&call.args).unwrap_or_default()).monospace().small());
                    match res {
                        Ok(r) => ui.label(RichText::new(r).small()),
                        Err(er) => ui.label(RichText::new(er).small().color(Color32::LIGHT_RED)),
                    };
                });
        }
    });
}

/// Minimal markdown: ``` code fences, **bold** lines, bullets.
fn render_markdownish(ui: &mut egui::Ui, text: &str) {
    let mut in_code = false;
    let mut code = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            if in_code {
                egui::Frame::new().fill(theme::BG1).corner_radius(4).inner_margin(6).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(code.trim_end()).monospace().size(11.5));
                });
                code.clear();
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code.push_str(line);
            code.push('\n');
            continue;
        }
        let t = line.trim_end();
        if let Some(h) = t.strip_prefix("### ").or_else(|| t.strip_prefix("## ")).or_else(|| t.strip_prefix("# ")) {
            ui.label(RichText::new(h).strong());
        } else if t.starts_with("**") && t.ends_with("**") && t.len() > 4 {
            ui.label(RichText::new(&t[2..t.len() - 2]).strong());
        } else {
            ui.label(t.replace("**", ""));
        }
    }
    if in_code && !code.is_empty() {
        ui.label(RichText::new(code.trim_end()).monospace().size(11.5));
    }
}
