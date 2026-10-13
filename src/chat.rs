//! Agent panel (right side) — minimal, Antigravity-style.

use crate::ai::agent::{self, AgentEvent, AgentHandle, Job, Mode, Role};
use crate::ai::client::Msg;
use crate::ai::models::{self, CLOUD_MODELS};
use crate::ai::tools::{self, ToolCall};
use crate::ai::web::SearchHit;
use crate::i18n::t;
use crate::theme;
use egui::{Color32, RichText};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;

#[derive(Clone, Serialize, Deserialize)]
pub enum Who {
    User,
    Agent(String),
    Info,
    Error,
    Memory,
    Todos,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub who: Who,
    pub model: String,
    pub text: String,
    pub tools: Vec<(ToolCall, Result<String, String>)>,
    #[serde(default)]
    pub sources: Vec<SearchHit>,
    #[serde(default)]
    pub items: Vec<String>,
    #[serde(default)]
    pub secs: f32,
    #[serde(skip)]
    pub started: Option<std::time::Instant>,
    #[serde(default)]
    pub feedback: Option<bool>,
}

impl Entry {
    fn new(who: Who, text: impl Into<String>) -> Self {
        Self {
            who,
            model: String::new(),
            text: text.into(),
            tools: vec![],
            sources: vec![],
            items: vec![],
            secs: 0.0,
            started: None,
            feedback: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub updated: u64,
    pub entries: Vec<Entry>,
    /// Compact memory for the LLM: (role, content).
    pub history: Vec<(String, String)>,
    /// Summary of compacted older turns.
    #[serde(default)]
    pub summary: String,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn chats_dir() -> std::path::PathBuf {
    let d = crate::config::data_dir().join("chats");
    let _ = std::fs::create_dir_all(&d);
    d
}

impl Conversation {
    fn new() -> Self {
        Self {
            id: format!("c{}", now()),
            ..Default::default()
        }
    }
    fn save(&self) {
        if self.entries.is_empty() {
            return;
        }
        if let Ok(s) = serde_json::to_string(self) {
            let _ = std::fs::write(chats_dir().join(format!("{}.json", self.id)), s);
        }
    }
    fn load(id: &str) -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(chats_dir().join(format!("{id}.json"))).ok()?)
            .ok()
    }
}

/// (id, title, updated) of saved conversations, newest first.
fn list_conversations() -> Vec<(String, String, u64)> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(chats_dir()) {
        for e in rd.flatten() {
            if let Ok(s) = std::fs::read_to_string(e.path()) {
                if let Ok(c) = serde_json::from_str::<Conversation>(&s) {
                    out.push((c.id, c.title, c.updated));
                }
            }
        }
    }
    out.sort_by(|a, b| b.2.cmp(&a.2));
    out
}

pub struct ChatPanel {
    pub conv: Conversation,
    pub input: String,
    pub mode: Mode,
    pub web: bool,
    handle: Option<AgentHandle>,
    pending_prompt: String,
    show_history: bool,
    history_cache: Vec<(String, String, u64)>,
    pub attach_selection: bool,
    focus_input: bool,
    /// A destructive tool call waiting for the user's permission.
    pending: Option<(ToolCall, std::sync::mpsc::Sender<Result<String, String>>)>,
    /// Index of the first entry of the current/last turn.
    turn_start: usize,
    show_review: bool,
    backlog: std::collections::VecDeque<AgentEvent>,
}

impl Default for ChatPanel {
    fn default() -> Self {
        Self {
            conv: Conversation::new(),
            input: String::new(),
            mode: Mode::Planning,
            web: true,
            handle: None,
            pending_prompt: String::new(),
            show_history: false,
            history_cache: vec![],
            attach_selection: true,
            focus_input: false,
            pending: None,
            turn_start: 0,
            show_review: false,
            backlog: Default::default(),
        }
    }
}

/// Requests the panel makes of the editor.
pub enum ChatRequest {
    Send,
    SelectModel(String),
    Settings,
    /// Answer to a permission prompt: (allow, always).
    Approve(bool, bool),
    UndoTurn,
    Feedback(bool),
}

const WORKFLOWS: &[(&str, &str, &str)] = &[
    (
        "/level",
        "Build a playable level: ",
        "Oynanabilir bir seviye kur: ",
    ),
    (
        "/game",
        "Make a complete small game: ",
        "Küçük ama tam bir oyun yap: ",
    ),
    (
        "/script",
        "Write a script for the selected object that ",
        "Seçili nesne için şu işi yapan bir betik yaz: ",
    ),
    (
        "/light",
        "Improve the lighting and materials of this scene for a ",
        "Bu sahnenin ışık ve malzemelerini şu havada güzelleştir: ",
    ),
    (
        "/fix",
        "Find and fix problems in the scene and scripts.",
        "Sahnedeki ve betiklerdeki sorunları bul ve düzelt.",
    ),
    (
        "/explain",
        "Explain how this scene works.",
        "Bu sahnenin nasıl çalıştığını açıkla.",
    ),
    ("/search", "Search the web for ", "Web'de ara: "),
];

impl ChatPanel {
    pub fn busy(&self) -> bool {
        self.handle.is_some()
    }

    pub fn focus(&mut self) {
        self.focus_input = true;
    }

    pub fn history_msgs(&self) -> Vec<Msg> {
        self.conv
            .history
            .iter()
            .rev()
            .take(6)
            .rev()
            .map(|(r, c)| {
                if r == "user" {
                    Msg::user(c)
                } else {
                    Msg::assistant(c)
                }
            })
            .collect()
    }

    pub fn start(&mut self, job: Job, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let prompt = job.prompt.clone();
        self.input.clear();
        if self.conv.title.is_empty() {
            self.conv.title = prompt.chars().take(60).collect();
        }
        self.turn_start = self.conv.entries.len();
        self.show_review = false;
        self.conv
            .entries
            .push(Entry::new(Who::User, prompt.clone()));
        self.pending_prompt = prompt;
        self.handle = Some(agent::spawn(job, ctx.clone()));
    }

    fn requeue(&mut self, rest: Vec<AgentEvent>) {
        self.backlog.extend(rest);
    }

    pub fn pending_name(&self) -> Option<String> {
        self.pending.as_ref().map(|(c, _)| c.name.clone())
    }

    /// Resolve the parked permission request.
    pub fn resolve(
        &mut self,
        allow: bool,
        mut exec: impl FnMut(&ToolCall) -> Result<String, String>,
    ) {
        let Some((call, reply)) = self.pending.take() else {
            return;
        };
        let r = if allow {
            exec(&call)
        } else {
            Err(t(
                "The user denied this action.",
                "Kullanıcı bu işlemi reddetti.",
            )
            .to_string())
        };
        let _ = reply.send(r.clone());
        if let Some(e) = self.agent_entry() {
            e.tools.push((call, r));
        }
    }

    /// Record Good/Bad on the latest answer; returns (prompt, answer) for memory.
    pub fn set_feedback(&mut self, good: bool) -> Option<(String, String)> {
        let i = self
            .conv
            .entries
            .iter()
            .rposition(|e| matches!(e.who, Who::Agent(_)))?;
        self.conv.entries[i].feedback = Some(good);
        let answer = tools::strip_tool_blocks(&self.conv.entries[i].text);
        let prompt = self.conv.entries[..i]
            .iter()
            .rev()
            .find(|e| matches!(e.who, Who::User))
            .map(|e| e.text.clone())
            .unwrap_or_default();
        self.conv.save();
        Some((prompt, answer))
    }

    pub fn summary(&self) -> String {
        self.conv.summary.clone()
    }

    /// Mutating tool calls of the latest turn.
    pub fn turn_changes(&self) -> Vec<(String, String)> {
        self.conv.entries[self.turn_start.min(self.conv.entries.len())..]
            .iter()
            .flat_map(|e| e.tools.iter())
            .filter(|(c, r)| tools::is_mutation(&c.name) && r.is_ok())
            .map(|(c, r)| {
                (
                    c.name.clone(),
                    r.as_ref()
                        .map(|s| s.lines().next().unwrap_or("").to_string())
                        .unwrap_or_default(),
                )
            })
            .collect()
    }

    pub fn stop(&mut self) {
        if let Some(h) = &self.handle {
            h.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn agent_entry(&mut self) -> Option<&mut Entry> {
        self.conv
            .entries
            .iter_mut()
            .rev()
            .find(|e| matches!(e.who, Who::Agent(_)))
    }

    /// Drain agent events. Scene tool calls are executed by `exec` (on the UI thread).
    pub fn poll(
        &mut self,
        mut exec: impl FnMut(&ToolCall) -> Result<String, String>,
        needs_approval: impl Fn(&ToolCall) -> bool,
    ) {
        if self.pending.is_some() {
            return;
        }
        let Some(h) = &self.handle else { return };
        let mut events: Vec<AgentEvent> = self.backlog.drain(..).collect();
        while let Ok(ev) = h.rx.try_recv() {
            events.push(ev);
        }
        let mut done = false;
        let mut events = events.into_iter();
        while let Some(ev) = events.next() {
            match ev {
                AgentEvent::Start { role, model } => {
                    let mut e = Entry::new(
                        Who::Agent(format!("{} {}", role.icon(), role_label(role))),
                        "",
                    );
                    e.model = model;
                    e.started = Some(std::time::Instant::now());
                    self.conv.entries.push(e);
                }
                AgentEvent::Delta { text } => {
                    if let Some(e) = self.conv.entries.last_mut() {
                        e.text.push_str(&text);
                    }
                }
                AgentEvent::End { model } => {
                    if let Some(e) = self
                        .conv
                        .entries
                        .iter_mut()
                        .rev()
                        .find(|e| matches!(e.who, Who::Agent(_)))
                    {
                        if !model.is_empty() {
                            e.model = models::display_name(&model);
                        }
                        if let Some(t0) = e.started {
                            e.secs = t0.elapsed().as_secs_f32();
                        }
                    }
                }
                AgentEvent::Tool { call, reply } if needs_approval(&call) => {
                    // Park it; remaining events are re-queued behind the approval.
                    self.pending = Some((call, reply));
                    let rest: Vec<AgentEvent> = events.collect();
                    self.requeue(rest);
                    return;
                }
                AgentEvent::Tool { call, reply } => {
                    let r = exec(&call);
                    let _ = reply.send(r.clone());
                    if let Some(e) = self.agent_entry() {
                        e.tools.push((call, r));
                    }
                }
                AgentEvent::ToolDone { call, result } => {
                    if let Some(e) = self.agent_entry() {
                        e.tools.push((call, result));
                    }
                }
                AgentEvent::Sources(hits) => {
                    if let Some(e) = self.agent_entry() {
                        e.sources.extend(hits);
                    }
                }
                AgentEvent::Todos(items) => {
                    let mut e = Entry::new(Who::Todos, "");
                    e.items = items
                        .into_iter()
                        .map(|(t, st)| format!("{st}|{t}"))
                        .collect();
                    // Replace the previous todo card of this turn.
                    let mut i = self.turn_start.min(self.conv.entries.len());
                    while i < self.conv.entries.len() {
                        if matches!(self.conv.entries[i].who, Who::Todos) {
                            self.conv.entries.remove(i);
                        } else {
                            i += 1;
                        }
                    }
                    self.conv.entries.push(e);
                }
                AgentEvent::Compacted(sum) => {
                    self.conv.summary = sum;
                    self.conv.entries.push(Entry::new(
                        Who::Info,
                        t(
                            "Older messages were summarized (compaction).",
                            "Eski mesajlar özetlendi (sıkıştırma).",
                        ),
                    ));
                }
                AgentEvent::Memories(items) => {
                    let mut e = Entry::new(Who::Memory, "");
                    e.items = items;
                    self.conv.entries.push(e);
                }
                AgentEvent::Info(s) => self.conv.entries.push(Entry::new(Who::Info, s)),
                AgentEvent::Error(s) => self.conv.entries.push(Entry::new(Who::Error, s)),
                AgentEvent::Done => done = true,
            }
        }
        if done {
            self.handle = None;
            let last = self
                .conv
                .entries
                .iter()
                .rev()
                .find(|e| matches!(e.who, Who::Agent(_)) && !e.text.is_empty())
                .map(|e| tools::strip_tool_blocks(&e.text))
                .unwrap_or_default();
            self.conv
                .history
                .push(("user".into(), std::mem::take(&mut self.pending_prompt)));
            self.conv.history.push(("assistant".into(), last));
            self.conv.updated = now();
            self.conv.save();
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        current_model: &str,
        local: &[String],
        entities: &[String],
        project: &str,
    ) -> Vec<ChatRequest> {
        let mut reqs = vec![];
        // ---------------------------------------------------------- header
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(t("Agent", "Ajan"))
                    .size(13.5)
                    .color(theme::BONE),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(RichText::new("…").size(15.0), |ui| {
                    if ui.button(t("⚙ AI settings", "⚙ AI ayarları")).clicked() {
                        reqs.push(ChatRequest::Settings);
                        ui.close_menu();
                    }
                    ui.checkbox(
                        &mut self.attach_selection,
                        t(
                            "Attach selected object as context",
                            "Seçili nesneyi bağlam olarak ekle",
                        ),
                    );
                });
                if icon_button(ui, "🕘", t("History", "Geçmiş"), self.show_history).clicked() {
                    self.show_history = !self.show_history;
                    if self.show_history {
                        self.history_cache = list_conversations();
                    }
                }
                if icon_button(ui, "+", t("New conversation", "Yeni sohbet"), false).clicked()
                    && !self.busy()
                {
                    self.conv.save();
                    self.conv = Conversation::new();
                    self.show_history = false;
                    self.focus_input = true;
                }
            });
        });
        ui.add_space(2.0);

        if self.show_history {
            self.history_ui(ui);
            return reqs;
        }

        // ---------------------------------------------------------- bottom: input card
        let empty = self.conv.entries.is_empty();
        if !empty {
            egui::TopBottomPanel::bottom("agent_input")
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    self.approval_card(ui, &mut reqs);
                    self.review_bar(ui, &mut reqs);
                    self.input_card(ui, current_model, local, entities, &mut reqs);
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(t(
                            "AI may make mistakes. Double-check all generated content.",
                            "AI hata yapabilir. Üretilen içeriği kontrol edin.",
                        ))
                        .size(10.5)
                        .weak(),
                    );
                });
        }

        // ---------------------------------------------------------- body
        if empty {
            ui.add_space((ui.available_height() * 0.30).max(10.0));
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(54.0, 54.0), egui::Sense::hover());
                theme::draw_logo(ui.painter(), r);
                ui.add_space(4.0);
                ui.label(RichText::new(project).size(18.0).color(theme::BONE));
                ui.add_space(14.0);
            });
            self.input_card(ui, current_model, local, entities, &mut reqs);
            ui.add_space(10.0);
            for (k, en, tr) in WORKFLOWS.iter().take(4) {
                if ui
                    .add(
                        egui::Label::new(
                            RichText::new(format!("{k}  {}", t(en, tr).trim_end_matches(": ")))
                                .weak(),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .clicked()
                {
                    self.input = t(en, tr).to_string();
                    self.focus_input = true;
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.label(
                    RichText::new(t(
                        "AI may make mistakes. Double-check all generated content.",
                        "AI hata yapabilir. Üretilen içeriği kontrol edin.",
                    ))
                    .size(10.5)
                    .weak(),
                );
            });
        } else {
            let busy = self.busy();
            // The last agent message of each turn is the "answer"; earlier ones are thoughts.
            let mut finals = vec![false; self.conv.entries.len()];
            let mut last_agent: Option<usize> = None;
            for (i, e) in self.conv.entries.iter().enumerate() {
                match e.who {
                    Who::User => {
                        if let Some(j) = last_agent.take() {
                            finals[j] = true;
                        }
                    }
                    Who::Agent(_) => last_agent = Some(i),
                    _ => {}
                }
            }
            if let Some(j) = last_agent {
                finals[j] = !busy;
            }
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    for (i, e) in self.conv.entries.iter().enumerate() {
                        if let Some(fb) = message(ui, e, i, finals[i] && e.tools.is_empty()) {
                            reqs.push(ChatRequest::Feedback(fb));
                        }
                    }
                    if self.busy() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(t("Working…", "Çalışıyor…")).weak());
                        });
                    }
                });
        }
        reqs
    }

    fn approval_card(&mut self, ui: &mut egui::Ui, reqs: &mut Vec<ChatRequest>) {
        let Some((call, _)) = &self.pending else {
            return;
        };
        egui::Frame::new()
            .fill(theme::BG2)
            .stroke(egui::Stroke::new(1.0_f32, theme::GOLD))
            .corner_radius(8)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(t("Permission needed", "İzin gerekiyor"))
                        .strong()
                        .color(theme::GOLD),
                );
                ui.label(format!(
                    "{} {}",
                    t("The agent wants to run", "Ajan şunu çalıştırmak istiyor:"),
                    call.name
                ));
                ui.label(
                    RichText::new(call.args.to_string().chars().take(200).collect::<String>())
                        .monospace()
                        .size(10.5)
                        .weak(),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(t("Allow", "İzin ver")).color(Color32::WHITE),
                            )
                            .fill(theme::ACCENT),
                        )
                        .clicked()
                    {
                        reqs.push(ChatRequest::Approve(true, false));
                    }
                    if ui.button(t("Always allow", "Her zaman izin ver")).clicked() {
                        reqs.push(ChatRequest::Approve(true, true));
                    }
                    if ui.button(t("Deny", "Reddet")).clicked() {
                        reqs.push(ChatRequest::Approve(false, false));
                    }
                });
            });
        ui.add_space(4.0);
    }

    /// "N changes · Review · Undo" bar for the latest turn.
    fn review_bar(&mut self, ui: &mut egui::Ui, reqs: &mut Vec<ChatRequest>) {
        let changes = self.turn_changes();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "📄 {} {}",
                    changes.len(),
                    t("changes", "değişiklik")
                ))
                .size(11.5)
                .weak(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !changes.is_empty()
                    && !self.busy()
                    && ui
                        .add(
                            egui::Button::new(RichText::new(t("↩ Undo", "↩ Geri al")).size(11.5))
                                .frame(false),
                        )
                        .clicked()
                {
                    reqs.push(ChatRequest::UndoTurn);
                }
                if ui
                    .add_enabled(
                        !changes.is_empty(),
                        egui::Button::new(
                            RichText::new(t("☰ Review Changes", "☰ Değişiklikleri incele"))
                                .size(11.5),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    self.show_review = !self.show_review;
                }
            });
        });
        if self.show_review {
            egui::Frame::new()
                .fill(theme::BG0)
                .corner_radius(6)
                .inner_margin(6)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .max_height(140.0)
                        .show(ui, |ui| {
                            for (name, res) in &changes {
                                ui.label(RichText::new(format!("• {name}: {res}")).size(11.0));
                            }
                        });
                });
        }
        ui.add_space(2.0);
    }

    fn history_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(t("Past conversations", "Geçmiş sohbetler")).strong());
        ui.separator();
        let mut open = None;
        let mut delete = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if self.history_cache.is_empty() {
                    ui.label(RichText::new(t("No conversations yet.", "Henüz sohbet yok.")).weak());
                }
                for (id, title, _) in &self.history_cache {
                    ui.horizontal(|ui| {
                        let title = if title.is_empty() {
                            "…"
                        } else {
                            title.as_str()
                        };
                        if ui
                            .add(
                                egui::Label::new(title)
                                    .truncate()
                                    .sense(egui::Sense::click()),
                            )
                            .clicked()
                        {
                            open = Some(id.clone());
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("🗑").clicked() {
                                delete = Some(id.clone());
                            }
                        });
                    });
                }
            });
        if let Some(id) = open {
            if let Some(c) = Conversation::load(&id) {
                self.conv.save();
                self.conv = c;
                self.show_history = false;
            }
        }
        if let Some(id) = delete {
            let _ = std::fs::remove_file(chats_dir().join(format!("{id}.json")));
            self.history_cache = list_conversations();
        }
    }

    fn input_card(
        &mut self,
        ui: &mut egui::Ui,
        current_model: &str,
        local: &[String],
        entities: &[String],
        reqs: &mut Vec<ChatRequest>,
    ) {
        let busy = self.busy();
        // Suggestion popups (@mention, /workflow) above the card.
        self.suggestions(ui, entities);
        egui::Frame::new()
            .fill(theme::BG0)
            .stroke(egui::Stroke::new(1.0_f32, theme::BG3))
            .corner_radius(10)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let te = egui::TextEdit::multiline(&mut self.input)
                    .id_salt("agent_input_text")
                    .frame(false)
                    .hint_text(t(
                        "Ask anything (Ctrl+L), @ to mention, / for workflows",
                        "Bir şey sor (Ctrl+L), @ ile nesne an, / iş akışları",
                    ))
                    .desired_rows(2)
                    .desired_width(f32::INFINITY);
                let r = ui.add(te);
                if self.focus_input {
                    r.request_focus();
                    self.focus_input = false;
                }
                let enter = r.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                if enter {
                    // Remove the newline the TextEdit just inserted.
                    while self.input.ends_with('\n') {
                        self.input.pop();
                    }
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.menu_button(RichText::new("+").size(15.0), |ui| {
                        ui.checkbox(
                            &mut self.attach_selection,
                            t("Selected object", "Seçili nesne"),
                        );
                        ui.separator();
                        for (k, en, tr) in WORKFLOWS {
                            if ui
                                .button(format!("{k}  {}", t(en, tr).trim_end_matches(": ")))
                                .clicked()
                            {
                                self.input = t(en, tr).to_string();
                                self.focus_input = true;
                                ui.close_menu();
                            }
                        }
                    });
                    ui.menu_button(
                        RichText::new(format!("⏷ {}", self.mode.label())).weak(),
                        |ui| {
                            ui.label(
                                RichText::new(t("Conversation mode", "Sohbet modu"))
                                    .small()
                                    .weak(),
                            );
                            for m in [Mode::Planning, Mode::Fast, Mode::Ask] {
                                if ui
                                    .selectable_label(self.mode == m, m.label())
                                    .on_hover_text(m.hint())
                                    .clicked()
                                {
                                    self.mode = m;
                                    ui.close_menu();
                                }
                                ui.label(RichText::new(m.hint()).size(10.5).weak());
                            }
                        },
                    );
                    ui.menu_button(
                        RichText::new(format!("⏷ {}", models::display_name(current_model))).weak(),
                        |ui| {
                            ui.set_min_width(220.0);
                            ui.label(RichText::new(t("Model", "Model")).small().weak());
                            for m in CLOUD_MODELS {
                                if ui
                                    .selectable_label(current_model == m.id, m.name)
                                    .on_hover_text(m.blurb)
                                    .clicked()
                                {
                                    reqs.push(ChatRequest::SelectModel(m.id.into()));
                                    ui.close_menu();
                                }
                            }
                            if !local.is_empty() {
                                ui.separator();
                                for m in local {
                                    if ui
                                        .selectable_label(
                                            current_model == m,
                                            models::display_name(m),
                                        )
                                        .clicked()
                                    {
                                        reqs.push(ChatRequest::SelectModel(m.clone()));
                                        ui.close_menu();
                                    }
                                }
                            }
                        },
                    );
                    let globe = RichText::new("🌐").color(if self.web {
                        theme::ACCENT
                    } else {
                        Color32::from_gray(110)
                    });
                    if ui
                        .add(egui::Button::new(globe).frame(false))
                        .on_hover_text(t("Web search from this device", "Bu cihazdan web araması"))
                        .clicked()
                    {
                        self.web = !self.web;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if busy {
                            if circle_button(ui, "■", true)
                                .on_hover_text(t("Stop", "Durdur"))
                                .clicked()
                            {
                                self.stop();
                            }
                        } else {
                            let ready = !self.input.trim().is_empty();
                            if (circle_button(ui, "▶", ready).clicked() || enter) && ready {
                                reqs.push(ChatRequest::Send);
                            }
                        }
                    });
                });
            });
    }

    fn suggestions(&mut self, ui: &mut egui::Ui, entities: &[String]) {
        let last_word = self
            .input
            .rsplit(char::is_whitespace)
            .next()
            .unwrap_or("")
            .to_string();
        let mut pick: Option<String> = None;
        if let Some(q) = last_word.strip_prefix('@') {
            let q = q.to_lowercase();
            let hits: Vec<&String> = entities
                .iter()
                .filter(|n| n.to_lowercase().replace(' ', "_").contains(&q))
                .take(6)
                .collect();
            if !hits.is_empty() {
                popup(ui, |ui| {
                    for h in hits {
                        if ui.selectable_label(false, format!("@ {h}")).clicked() {
                            pick = Some(format!("@{} ", h.replace(' ', "_")));
                        }
                    }
                });
            }
        } else if self.input.starts_with('/') && !self.input.contains(' ') {
            let q = self.input.to_lowercase();
            popup(ui, |ui| {
                for (k, en, tr) in WORKFLOWS.iter().filter(|w| w.0.starts_with(&q)) {
                    if ui
                        .selectable_label(
                            false,
                            format!("{k}  {}", t(en, tr).trim_end_matches(": ")),
                        )
                        .clicked()
                    {
                        self.input = t(en, tr).to_string();
                        self.focus_input = true;
                    }
                }
            });
        }
        if let Some(p) = pick {
            let cut = self.input.len() - last_word.len();
            self.input.truncate(cut);
            self.input.push_str(&p);
            self.focus_input = true;
        }
    }
}

fn role_label(r: Role) -> &'static str {
    match r {
        Role::Assistant => "Forge AI",
        Role::Planner => t("Planner", "Planlayıcı"),
        Role::Builder => t("Builder", "Kurucu"),
        Role::Coder => t("Coder", "Kodlayıcı"),
        Role::Reviewer => t("Reviewer", "Denetçi"),
        Role::Explore => "Router",
    }
}

fn popup(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(theme::BG2)
        .corner_radius(8)
        .inner_margin(6)
        .stroke(egui::Stroke::new(1.0_f32, theme::BG3))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn icon_button(ui: &mut egui::Ui, icon: &str, hover: &str, active: bool) -> egui::Response {
    let c = if active {
        theme::ACCENT
    } else {
        Color32::from_gray(170)
    };
    ui.add(egui::Button::new(RichText::new(icon).size(14.0).color(c)).frame(false))
        .on_hover_text(hover)
}

fn circle_button(ui: &mut egui::Ui, icon: &str, enabled: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
    let fill = if !enabled {
        theme::BG3
    } else if r.hovered() {
        theme::ACCENT
    } else {
        Color32::from_gray(200)
    };
    ui.painter().circle_filled(rect.center(), 13.0, fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(11.0),
        theme::BG0,
    );
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn tool_verb(name: &str) -> (&'static str, &'static str) {
    match name {
        "get_scene" | "read_file" | "list_files" | "grep" | "memory_search" => {
            ("📄", t("Analyzed", "İncelendi"))
        }
        "web_search" => ("🌐", t("Searched", "Arandı")),
        "fetch_url" => ("🌐", t("Read", "Okundu")),
        "delete_entity" | "clear_scene" | "delete_file" => ("🗑", t("Deleted", "Silindi")),
        "task" | "ask_coder" => ("🔭", t("Delegated", "Devredildi")),
        "todo_write" => ("☰", t("Planned", "Planlandı")),
        "remember" => ("💭", t("Remembered", "Hatırlandı")),
        _ => ("✏", t("Edited", "Düzenlendi")),
    }
}

/// Renders one entry; returns Some(feedback) when the user clicks Good/Bad.
fn message(ui: &mut egui::Ui, e: &Entry, idx: usize, is_final: bool) -> Option<bool> {
    let mut fb = None;
    match &e.who {
        Who::User => {
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(theme::BG2)
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&e.text).color(theme::BONE));
                });
            ui.add_space(6.0);
        }
        Who::Memory => {
            egui::CollapsingHeader::new(
                RichText::new(format!(
                    "💭 {} {}",
                    e.items.len(),
                    t("memories recalled", "anı hatırlandı")
                ))
                .size(12.0)
                .weak(),
            )
            .id_salt(("mem", idx))
            .show(ui, |ui| {
                for it in &e.items {
                    ui.label(RichText::new(format!("• {it}")).small().weak());
                }
            });
        }
        Who::Todos => {
            egui::Frame::new()
                .fill(theme::BG0)
                .corner_radius(6)
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(format!("☰ {}", t("Task", "Görev")))
                            .size(12.0)
                            .color(theme::BONE),
                    );
                    for it in &e.items {
                        let (st, text) = it.split_once('|').unwrap_or(("pending", it));
                        let (mark, c) = match st {
                            "completed" | "done" => ("☑", Color32::from_rgb(120, 210, 140)),
                            "in_progress" => ("🎨", theme::GOLD),
                            _ => ("☐", Color32::from_gray(150)),
                        };
                        ui.label(RichText::new(format!("{mark} {text}")).size(12.0).color(c));
                    }
                });
        }
        Who::Info => {
            ui.label(RichText::new(format!("ℹ {}", e.text)).size(11.5).weak());
        }
        Who::Error => {
            ui.label(RichText::new(format!("⚠ {}", e.text)).color(Color32::LIGHT_RED));
        }
        Who::Agent(label) => {
            let text = tools::strip_tool_blocks(&e.text);
            if is_final {
                render_markdownish(ui, &text);
                sources(ui, e);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{label} · {}", e.model))
                            .size(10.5)
                            .weak(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let bad = RichText::new(format!("{} 👎", t("Bad", "Kötü")))
                            .size(11.0)
                            .color(if e.feedback == Some(false) {
                                theme::ACCENT
                            } else {
                                Color32::from_gray(130)
                            });
                        if ui.add(egui::Button::new(bad).frame(false)).clicked() {
                            fb = Some(false);
                        }
                        let good = RichText::new(format!("{} 👍", t("Good", "İyi")))
                            .size(11.0)
                            .color(if e.feedback == Some(true) {
                                theme::ACCENT
                            } else {
                                Color32::from_gray(130)
                            });
                        if ui.add(egui::Button::new(good).frame(false)).clicked() {
                            fb = Some(true);
                        }
                    });
                });
                ui.add_space(6.0);
                return fb;
            }
            let head = if e.started.is_some() && e.secs == 0.0 {
                format!("{} {}…", label, t("thinking", "düşünüyor"))
            } else {
                format!("{} {:.0}s", t("Thought for", "Düşündü:"), e.secs.max(1.0))
            };
            egui::CollapsingHeader::new(
                RichText::new(head)
                    .size(12.0)
                    .color(Color32::from_gray(170)),
            )
            .id_salt(("thought", idx))
            .default_open(false)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!("{label} · {}", e.model))
                        .size(10.5)
                        .color(theme::ACCENT),
                );
                if !text.is_empty() {
                    render_markdownish(ui, &text);
                }
            });
            for (k, (call, res)) in e.tools.iter().enumerate() {
                let (icon, verb) = tool_verb(&call.name);
                let target = call
                    .args
                    .get("target")
                    .or_else(|| call.args.get("name"))
                    .or_else(|| call.args.get("query"))
                    .or_else(|| call.args.get("path"))
                    .or_else(|| call.args.get("url"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.chars().take(36).collect::<String>())
                    .unwrap_or_else(|| call.name.clone());
                let c = if res.is_ok() {
                    Color32::from_gray(185)
                } else {
                    Color32::LIGHT_RED
                };
                egui::CollapsingHeader::new(
                    RichText::new(format!("{icon} {verb}  {target}"))
                        .size(12.0)
                        .color(c),
                )
                .id_salt(("tool", idx, k))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} {}",
                            call.name,
                            serde_json::to_string_pretty(&call.args).unwrap_or_default()
                        ))
                        .monospace()
                        .size(10.5),
                    );
                    match res {
                        Ok(r) => ui.label(
                            RichText::new(r.chars().take(1500).collect::<String>()).size(11.0),
                        ),
                        Err(er) => ui.label(RichText::new(er).size(11.0).color(Color32::LIGHT_RED)),
                    };
                });
            }
            sources(ui, e);
        }
    }
    fb
}

fn sources(ui: &mut egui::Ui, e: &Entry) {
    if e.sources.is_empty() {
        return;
    }
    ui.label(RichText::new(t("Sources", "Kaynaklar")).size(11.0).weak());
    for s in e.sources.iter().take(6) {
        let host = s.url.split('/').nth(2).unwrap_or(&s.url);
        ui.horizontal(|ui| {
            ui.hyperlink_to(
                RichText::new(s.title.chars().take(52).collect::<String>()).size(11.5),
                &s.url,
            );
            ui.label(RichText::new(host).size(10.0).weak());
        });
    }
}

/// Minimal markdown: ``` code fences, headings, **bold** lines, [links](url).
fn render_markdownish(ui: &mut egui::Ui, text: &str) {
    let mut in_code = false;
    let mut code = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            if in_code {
                egui::Frame::new()
                    .fill(theme::BG0)
                    .corner_radius(6)
                    .inner_margin(6)
                    .show(ui, |ui| {
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
        let tl = line.trim_end();
        if let Some(h) = tl
            .strip_prefix("### ")
            .or_else(|| tl.strip_prefix("## "))
            .or_else(|| tl.strip_prefix("# "))
        {
            ui.label(RichText::new(h).strong());
        } else if let (Some(a), Some(b)) = (tl.find("]("), tl.rfind(')')) {
            // Single link per line is common in source lists.
            if let Some(s) = tl[..a].rfind('[') {
                if b > a {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(tl[..s].replace("**", ""));
                        ui.hyperlink_to(&tl[s + 1..a], &tl[a + 2..b]);
                        ui.label(tl[b + 1..].replace("**", ""));
                    });
                    continue;
                }
            }
            ui.label(tl.replace("**", ""));
        } else {
            ui.label(tl.replace("**", ""));
        }
    }
    if in_code && !code.is_empty() {
        ui.label(RichText::new(code.trim_end()).monospace().size(11.5));
    }
}
