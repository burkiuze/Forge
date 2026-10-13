//! The Forge editor: menus, hierarchy, library, viewport, inspector, console, script editor and AI chat.

use crate::ai::agent::Job;
use crate::ai::tools::{self, ToolCall};
use crate::chat::{ChatPanel, ChatRequest};
use crate::config::{Secrets, Settings};
use crate::library::{LibAction, Library};
use crate::render::{self, CameraView, DrawList, GlRenderer, OrbitCamera};
use crate::runtime::{self, Runtime};
use crate::scene::{CameraComp, EntityId, Light, MeshKind, Rigidbody, Scene};
use crate::templates::Template;
use crate::theme::{self, ACCENT};
use crate::wizard::Wizard;
use eframe::egui_glow;
use egui::{Color32, RichText};
use glam::Vec3;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(PartialEq, Clone, Copy)]
enum LeftTab {
    Hierarchy,
    Library,
}

#[derive(PartialEq, Clone, Copy)]
enum BottomTab {
    Console,
    Script,
}

#[derive(PartialEq, Clone, Copy)]
enum ViewTab {
    Scene,
    Game,
}

#[derive(Clone, Copy, PartialEq)]
enum LogLevel {
    Info,
    Warn,
    Error,
    Ai,
}

struct GizmoDrag {
    axis: usize,
    start_pos: Vec3,
    accumulated: f32,
}

#[derive(Default)]
struct Dialogs {
    open: Option<String>,
    save_as: Option<String>,
    build: Option<String>,
    settings: bool,
    scripting_help: bool,
    about: bool,
    script_goal: String,
}

pub struct ForgeApp {
    settings: Settings,
    secrets: Secrets,
    scene: Scene,
    scene_path: Option<PathBuf>,
    dirty: bool,
    selected: Option<EntityId>,
    undo: Vec<String>,
    redo: Vec<String>,
    editing: bool,
    cam: OrbitCamera,
    renderer: Option<Arc<Mutex<GlRenderer>>>,
    runtime: Option<Runtime>,
    view_tab: ViewTab,
    left_tab: LeftTab,
    bottom_tab: BottomTab,
    console: Vec<(LogLevel, String)>,
    chat: ChatPanel,
    library: Library,
    wizard: Option<Wizard>,
    dialogs: Dialogs,
    gizmo: Option<GizmoDrag>,
    hierarchy_filter: String,
    player_mode: bool,
    last_frame: Instant,
    frame_ms: f32,
    last_title: String,
}

impl ForgeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, player_scene: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        let mut console = vec![];
        let renderer = match cc.gl.as_ref().map(|gl| GlRenderer::new(gl)) {
            Some(Ok(r)) => Some(Arc::new(Mutex::new(r))),
            Some(Err(e)) => {
                console.push((LogLevel::Error, format!("Renderer init failed: {e}")));
                None
            }
            None => None,
        };
        let settings = Settings::load();
        let secrets = Secrets::load();
        let mut library = Library::default();
        library.refresh_installed(&settings);

        let mut app = Self {
            wizard: (!settings.setup_done && player_scene.is_none()).then(|| Wizard::new(&settings, &secrets)),
            settings,
            secrets,
            scene: Scene::starter(),
            scene_path: None,
            dirty: false,
            selected: None,
            undo: vec![],
            redo: vec![],
            editing: false,
            cam: OrbitCamera::default(),
            renderer,
            runtime: None,
            view_tab: ViewTab::Scene,
            left_tab: LeftTab::Hierarchy,
            bottom_tab: BottomTab::Console,
            console,
            chat: ChatPanel::default(),
            library,
            dialogs: Dialogs::default(),
            gizmo: None,
            hierarchy_filter: String::new(),
            player_mode: player_scene.is_some(),
            last_frame: Instant::now(),
            frame_ms: 0.0,
            last_title: String::new(),
        };
        if let Some(p) = player_scene {
            match Scene::load(&p) {
                Ok(s) => app.scene = s,
                Err(e) => app.log(LogLevel::Error, format!("Could not load game: {e}")),
            }
            app.play();
        } else if let Some(p) = app.settings.last_scene.clone() {
            if p.is_file() {
                app.open_scene(p);
            }
        }
        if app.secrets.openrouter_key.is_empty() && !app.player_mode {
            app.log(LogLevel::Warn, "No OpenRouter key yet — add it in AI ▸ Settings or run the Setup Wizard.".into());
        }
        app.log(LogLevel::Info, format!("Forge {} ready. Data dir: {}", env!("CARGO_PKG_VERSION"), crate::config::data_dir().display()));
        app
    }

    fn log(&mut self, level: LogLevel, msg: String) {
        self.console.push((level, msg));
        if self.console.len() > 2000 {
            self.console.drain(0..500);
        }
    }

    // ------------------------------------------------------------ scene ops

    fn push_undo(&mut self) {
        self.undo.push(serde_json::to_string(&self.scene).unwrap());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(serde_json::to_string(&self.scene).unwrap());
            self.scene = serde_json::from_str(&s).unwrap();
            self.dirty = true;
        }
    }

    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(serde_json::to_string(&self.scene).unwrap());
            self.scene = serde_json::from_str(&s).unwrap();
            self.dirty = true;
        }
    }

    fn create(&mut self, name: &str, mesh: MeshKind) -> EntityId {
        self.push_undo();
        let id = self.scene.spawn(name, mesh);
        let e = self.scene.get_mut(id).unwrap();
        // Drop new objects in front of the editor camera, resting on the ground.
        let t = self.cam.target;
        e.transform.position = Vec3::new(t.x.round(), if mesh == MeshKind::Plane { 0.0 } else { 0.5 }, t.z.round());
        if mesh != MeshKind::None {
            e.color = [0.85, 0.85, 0.85];
        }
        self.selected = Some(id);
        id
    }

    fn delete_selected(&mut self) {
        if let Some(id) = self.selected.take() {
            self.push_undo();
            self.scene.remove(id);
        }
    }

    fn duplicate_selected(&mut self) {
        if let Some(id) = self.selected {
            self.push_undo();
            self.selected = self.scene.duplicate(id);
        }
    }

    fn new_scene(&mut self, t: Template) {
        self.push_undo();
        self.scene = t.build();
        self.scene_path = None;
        self.selected = None;
    }

    fn open_scene(&mut self, p: PathBuf) {
        match Scene::load(&p) {
            Ok(s) => {
                self.scene = s;
                self.scene_path = Some(p.clone());
                self.selected = None;
                self.undo.clear();
                self.redo.clear();
                self.dirty = false;
                self.settings.last_scene = Some(p.clone());
                self.settings.save();
                self.log(LogLevel::Info, format!("Opened {}", p.display()));
            }
            Err(e) => self.log(LogLevel::Error, format!("Open failed: {e}")),
        }
    }

    fn save(&mut self) {
        match self.scene_path.clone() {
            Some(p) => self.save_to(p),
            None => {
                self.dialogs.save_as = Some(
                    self.settings.projects_dir.join(&self.scene.name).join("main.scene.json").display().to_string(),
                )
            }
        }
    }

    fn save_to(&mut self, p: PathBuf) {
        match self.scene.save(&p) {
            Ok(()) => {
                self.scene_path = Some(p.clone());
                self.dirty = false;
                self.settings.last_scene = Some(p.clone());
                self.settings.save();
                self.log(LogLevel::Info, format!("Saved {}", p.display()));
            }
            Err(e) => self.log(LogLevel::Error, format!("Save failed: {e}")),
        }
    }

    /// Export a standalone game: copy the Forge binary + scene into a folder.
    fn build_game(&mut self, dir: PathBuf) {
        let res = (|| -> std::io::Result<PathBuf> {
            std::fs::create_dir_all(&dir)?;
            let exe = std::env::current_exe()?;
            let name: String = self.scene.name.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect();
            let name = if name.is_empty() { "Game".to_string() } else { name };
            let target = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
            std::fs::copy(&exe, &target)?;
            self.scene.save(&dir.join("game.scene.json"))?;
            Ok(target)
        })();
        match res {
            Ok(t) => self.log(LogLevel::Info, format!("Game built: {}", t.display())),
            Err(e) => self.log(LogLevel::Error, format!("Build failed: {e}")),
        }
    }

    // ------------------------------------------------------------ play mode

    fn play(&mut self) {
        if self.runtime.is_some() {
            return;
        }
        let (rt, errors) = Runtime::new(self.scene.clone());
        for e in errors {
            self.log(LogLevel::Error, e);
        }
        self.runtime = Some(rt);
        self.view_tab = ViewTab::Game;
        if !self.player_mode {
            self.log(LogLevel::Info, "▶ Play".into());
        }
    }

    fn stop(&mut self) {
        if self.runtime.take().is_some() {
            self.view_tab = ViewTab::Scene;
            self.log(LogLevel::Info, "■ Stop — scene restored".into());
        }
    }

    fn tick_runtime(&mut self, ctx: &egui::Context, dt: f32) {
        let typing = ctx.wants_keyboard_input();
        let Some(rt) = &mut self.runtime else { return };
        let (down, pressed) = if typing { Default::default() } else { runtime::collect_input(ctx) };
        rt.set_input(down, pressed);
        let errors = rt.step(dt);
        let logs = rt.take_logs();
        for e in errors {
            self.log(LogLevel::Error, e);
        }
        for l in logs {
            self.log(LogLevel::Info, l);
        }
        ctx.request_repaint();
    }

    // ------------------------------------------------------------ AI

    fn select_model(&mut self, id: String, ctx: &egui::Context) {
        if let Some(file) = id.strip_prefix("local:") {
            if self.library.llama_running_model().as_deref() != Some(file) {
                if let Err(e) = self.library.start_llama(&self.settings, file, ctx) {
                    self.log(LogLevel::Error, e);
                    return;
                }
                self.log(LogLevel::Ai, format!("Starting local model {file}…"));
            }
        }
        self.settings.selected_model = id.clone();
        self.settings.save();
        self.log(LogLevel::Ai, format!("Model: {}", crate::ai::models::display_name(&id)));
    }

    fn send_chat(&mut self, ctx: &egui::Context) {
        if self.runtime.is_some() {
            self.stop();
        }
        let scene_json = self.scene.summary();
        let selected = self.selected.and_then(|s| self.scene.get(s)).map(|e| e.name.clone());
        let settings = self.settings.clone();
        let key = self.secrets.openrouter_key.clone();
        let prompt = self.chat.input.trim().to_string();
        if prompt.is_empty() {
            return;
        }
        self.push_undo(); // one undo step for the whole agent run
        self.chat.start(
            |history, mode| Job { prompt, mode, history, scene_json, selected, settings, openrouter_key: key },
            ctx,
        );
    }

    fn exec_tool(&mut self, call: &ToolCall) -> Result<String, String> {
        let r = tools::execute(&mut self.scene, call);
        match &r {
            Ok(o) => {
                self.dirty = true;
                if o.select.is_some() {
                    self.selected = o.select;
                }
                self.log(LogLevel::Ai, format!("tool {} ✔", call.name));
            }
            Err(e) => self.log(LogLevel::Warn, format!("tool {} ✘ {e}", call.name)),
        }
        r.map(|o| o.result)
    }

    // ------------------------------------------------------------ UI: menus & toolbar

    fn menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::menu::bar(ui, |ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            theme::draw_logo(ui.painter(), r);
            ui.menu_button("File", |ui| {
                ui.menu_button("New Scene", |ui| {
                    for t in Template::ALL {
                        if ui.button(t.label()).clicked() {
                            self.new_scene(t);
                            ui.close_menu();
                        }
                    }
                });
                if ui.button("Open…").clicked() {
                    self.dialogs.open = Some(self.settings.projects_dir.display().to_string());
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Save").shortcut_text("Ctrl+S")).clicked() {
                    self.save();
                    ui.close_menu();
                }
                if ui.button("Save As…").clicked() {
                    self.dialogs.save_as = Some(
                        self.scene_path.clone().unwrap_or_else(|| self.settings.projects_dir.join("Untitled").join("main.scene.json")).display().to_string(),
                    );
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Build Game…").clicked() {
                    self.dialogs.build = Some(self.settings.projects_dir.join("Builds").join(&self.scene.name).display().to_string());
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo").shortcut_text("Ctrl+Z")).clicked() {
                    self.undo();
                    ui.close_menu();
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo").shortcut_text("Ctrl+Y")).clicked() {
                    self.redo();
                    ui.close_menu();
                }
                ui.separator();
                if ui.add_enabled(self.selected.is_some(), egui::Button::new("Duplicate").shortcut_text("Ctrl+D")).clicked() {
                    self.duplicate_selected();
                    ui.close_menu();
                }
                if ui.add_enabled(self.selected.is_some(), egui::Button::new("Delete").shortcut_text("Del")).clicked() {
                    self.delete_selected();
                    ui.close_menu();
                }
            });
            ui.menu_button("GameObject", |ui| self.create_menu(ui));
            ui.menu_button("AI", |ui| {
                if ui.button("⚙ AI Settings…").clicked() {
                    self.dialogs.settings = true;
                    ui.close_menu();
                }
                if ui.button("📚 Model Library").clicked() {
                    self.left_tab = LeftTab::Library;
                    ui.close_menu();
                }
                if ui.button("🧙 Setup Wizard…").clicked() {
                    self.wizard = Some(Wizard::new(&self.settings, &self.secrets));
                    ui.close_menu();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Scripting reference").clicked() {
                    self.dialogs.scripting_help = true;
                    ui.close_menu();
                }
                if ui.button("About Forge").clicked() {
                    self.dialogs.about = true;
                    ui.close_menu();
                }
            });

            // Play controls, centered.
            let mid = ui.max_rect().center().x;
            let w = 120.0;
            let cur = ui.cursor().min.x;
            if mid - w / 2.0 > cur {
                ui.add_space(mid - w / 2.0 - cur);
            }
            let playing = self.runtime.is_some();
            let paused = self.runtime.as_ref().map_or(false, |r| r.paused);
            let play_btn = egui::Button::new(RichText::new(if playing { "■" } else { "▶" }).size(15.0).color(if playing { ACCENT } else { theme::BONE }))
                .fill(if playing { theme::BG3 } else { theme::BG2 })
                .min_size(egui::vec2(34.0, 22.0));
            if ui.add(play_btn).on_hover_text("Play / Stop (Ctrl+P)").clicked() {
                if playing { self.stop() } else { self.play() }
            }
            if ui
                .add_enabled(playing, egui::Button::new(RichText::new("⏸").size(13.0).color(if paused { ACCENT } else { theme::BONE })).min_size(egui::vec2(34.0, 22.0)))
                .on_hover_text("Pause")
                .clicked()
            {
                if let Some(r) = &mut self.runtime {
                    r.paused = !r.paused;
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{:.1} ms", self.frame_ms)).small().weak());
                ui.label(RichText::new(crate::ai::models::display_name(&self.settings.selected_model)).small().color(theme::GOLD));
                ui.label(RichText::new("model:").small().weak());
            });
        });
    }

    fn create_menu(&mut self, ui: &mut egui::Ui) {
        for (label, mesh) in [
            ("Create Empty", MeshKind::None),
            ("Cube", MeshKind::Cube),
            ("Sphere", MeshKind::Sphere),
            ("Plane", MeshKind::Plane),
            ("Cylinder", MeshKind::Cylinder),
            ("Cone", MeshKind::Cone),
            ("Capsule", MeshKind::Capsule),
        ] {
            if ui.button(label).clicked() {
                let name = if mesh == MeshKind::None { "GameObject" } else { mesh.label() };
                self.create(name, mesh);
                ui.close_menu();
            }
        }
        ui.separator();
        if ui.button("Camera").clicked() {
            let id = self.create("Camera", MeshKind::None);
            let e = self.scene.get_mut(id).unwrap();
            e.camera = Some(CameraComp::default());
            e.transform.position = Vec3::new(0.0, 4.0, 10.0);
            e.transform.rotation = Vec3::new(-15.0, 0.0, 0.0);
            ui.close_menu();
        }
        if ui.button("Directional Light").clicked() {
            let id = self.create("Light", MeshKind::None);
            let e = self.scene.get_mut(id).unwrap();
            e.light = Some(Light::default());
            e.transform.position.y = 8.0;
            e.transform.rotation = Vec3::new(-50.0, 30.0, 0.0);
            ui.close_menu();
        }
    }

    // ------------------------------------------------------------ UI: hierarchy

    fn hierarchy_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.menu_button("+", |ui| self.create_menu(ui));
            ui.add(egui::TextEdit::singleline(&mut self.hierarchy_filter).hint_text("🔍 Search").desired_width(f32::INFINITY));
        });
        ui.label(RichText::new(format!("🎬 {}{}", self.scene.name, if self.dirty { " *" } else { "" })).strong());
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let filter = self.hierarchy_filter.to_lowercase();
            if filter.is_empty() {
                for id in self.scene.children(None) {
                    self.tree_node(ui, id, 0);
                }
            } else {
                let ids: Vec<EntityId> =
                    self.scene.entities.iter().filter(|e| e.name.to_lowercase().contains(&filter)).map(|e| e.id).collect();
                for id in ids {
                    self.entity_row(ui, id, 0);
                }
            }
        });
    }

    fn tree_node(&mut self, ui: &mut egui::Ui, id: EntityId, depth: usize) {
        self.entity_row(ui, id, depth);
        if depth < 16 {
            for c in self.scene.children(Some(id)) {
                self.tree_node(ui, c, depth + 1);
            }
        }
    }

    fn entity_row(&mut self, ui: &mut egui::Ui, id: EntityId, depth: usize) {
        let Some(e) = self.scene.get(id) else { return };
        let icon = if e.camera.is_some() {
            "🎥"
        } else if e.light.is_some() {
            "☀"
        } else if !e.script.is_empty() {
            "📜"
        } else if e.mesh == MeshKind::None {
            "○"
        } else {
            "◼"
        };
        let mut text = RichText::new(format!("{icon} {}", e.name));
        if !e.active {
            text = text.weak();
        }
        let sel = self.selected == Some(id);
        let r = ui
            .horizontal(|ui| {
                ui.add_space(depth as f32 * 12.0);
                ui.selectable_label(sel, text)
            })
            .inner;
        if r.clicked() {
            self.selected = Some(id);
        }
        if r.double_clicked() {
            self.focus(id);
        }
        r.context_menu(|ui| {
            self.selected = Some(id);
            if ui.button("Focus").clicked() {
                self.focus(id);
                ui.close_menu();
            }
            if ui.button("Duplicate").clicked() {
                self.duplicate_selected();
                ui.close_menu();
            }
            if ui.button("Delete").clicked() {
                self.delete_selected();
                ui.close_menu();
            }
            ui.separator();
            if ui.button("✨ Ask Forge AI about this").clicked() {
                let name = self.scene.get(id).map(|e| e.name.clone()).unwrap_or_default();
                self.chat.input = format!("'{name}' için ");
                ui.close_menu();
            }
        });
    }

    fn focus(&mut self, id: EntityId) {
        let m = self.scene.world_matrix(id);
        let (s, _, p) = m.to_scale_rotation_translation();
        self.cam.target = p;
        self.cam.distance = (s.max_element() * 3.0).clamp(3.0, 80.0);
    }

    // ------------------------------------------------------------ UI: inspector

    fn inspector_ui(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("Select an object to inspect").weak()));
            self.environment_ui(ui);
            return;
        };
        let Some(before) = self.scene.get(id).cloned() else {
            self.selected = None;
            return;
        };
        let mut e = before.clone();
        let mut changed = false;
        let mut remove = false;
        let entity_names: Vec<String> = self.scene.entities.iter().filter(|x| x.id != id).map(|x| x.name.clone()).collect();

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut e.active, "").changed();
                changed |= ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(f32::INFINITY)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Tag");
                changed |= ui.add(egui::TextEdit::singleline(&mut e.tag).desired_width(90.0)).changed();
                ui.label(RichText::new(format!("id {}", e.id)).small().weak());
            });
            section(ui, "Transform", |ui| {
                egui::Grid::new("tf").num_columns(4).spacing([4.0, 4.0]).show(ui, |ui| {
                    for (label, v, speed) in [
                        ("Position", &mut e.transform.position, 0.05),
                        ("Rotation", &mut e.transform.rotation, 0.5),
                        ("Scale", &mut e.transform.scale, 0.02),
                    ] {
                        ui.label(label);
                        for (c, axis) in [(Color32::from_rgb(230, 80, 70), &mut v.x), (Color32::from_rgb(110, 200, 90), &mut v.y), (Color32::from_rgb(80, 140, 240), &mut v.z)] {
                            let r = ui.add(egui::DragValue::new(axis).speed(speed).max_decimals(3).custom_formatter(|n, _| format!("{n:.2}")));
                            ui.painter().line_segment([r.rect.left_top(), r.rect.left_bottom()], egui::Stroke::new(2.0_f32, c));
                            changed |= r.changed();
                        }
                        ui.end_row();
                    }
                });
            });
            section(ui, "Mesh Renderer", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Mesh");
                    egui::ComboBox::from_id_salt("mesh").selected_text(e.mesh.label()).show_ui(ui, |ui| {
                        for m in MeshKind::ALL {
                            changed |= ui.selectable_value(&mut e.mesh, m, m.label()).changed();
                        }
                    });
                    ui.label("Color");
                    changed |= ui.color_edit_button_rgb(&mut e.color).changed();
                });
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut e.collider, "Collider").changed();
                    changed |= ui.checkbox(&mut e.trigger, "Is Trigger").changed();
                });
            });
            if let Some(rb) = &mut e.rigidbody {
                let mut keep = true;
                section_removable(ui, "Rigidbody", &mut keep, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Mass");
                        changed |= ui.add(egui::DragValue::new(&mut rb.mass).speed(0.05).range(0.01..=1000.0)).changed();
                        ui.label("Bounce");
                        changed |= ui.add(egui::DragValue::new(&mut rb.bounciness).speed(0.01).range(0.0..=1.0)).changed();
                    });
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut rb.use_gravity, "Use Gravity").changed();
                        changed |= ui.checkbox(&mut rb.is_static, "Static").changed();
                    });
                });
                if !keep {
                    e.rigidbody = None;
                    changed = true;
                }
            }
            if let Some(l) = &mut e.light {
                let mut keep = true;
                section_removable(ui, "Directional Light", &mut keep, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Color");
                        changed |= ui.color_edit_button_rgb(&mut l.color).changed();
                        ui.label("Intensity");
                        changed |= ui.add(egui::DragValue::new(&mut l.intensity).speed(0.02).range(0.0..=8.0)).changed();
                    });
                });
                if !keep {
                    e.light = None;
                    changed = true;
                }
            }
            if let Some(c) = &mut e.camera {
                let mut keep = true;
                section_removable(ui, "Camera", &mut keep, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("FOV");
                        changed |= ui.add(egui::Slider::new(&mut c.fov, 20.0..=120.0)).changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Follow");
                        egui::ComboBox::from_id_salt("follow")
                            .selected_text(if c.follow.is_empty() { "(none)" } else { c.follow.as_str() })
                            .show_ui(ui, |ui| {
                                changed |= ui.selectable_value(&mut c.follow, String::new(), "(none)").changed();
                                for n in &entity_names {
                                    changed |= ui.selectable_value(&mut c.follow, n.clone(), n).changed();
                                }
                            });
                    });
                });
                if !keep {
                    e.camera = None;
                    changed = true;
                }
            }
            section(ui, "Script (Rhai)", |ui| {
                if e.script.is_empty() {
                    ui.small("No script.");
                } else {
                    ui.small(format!("{} lines", e.script.lines().count()));
                }
                ui.horizontal(|ui| {
                    if ui.button("✏ Edit").clicked() {
                        self.bottom_tab = BottomTab::Script;
                    }
                    if !e.script.is_empty() && ui.button("✖ Remove").clicked() {
                        e.script.clear();
                        changed = true;
                    }
                });
            });
            ui.add_space(6.0);
            ui.vertical_centered(|ui| {
                ui.menu_button("Add Component", |ui| {
                    if ui.add_enabled(e.rigidbody.is_none(), egui::Button::new("Rigidbody")).clicked() {
                        e.rigidbody = Some(Rigidbody::default());
                        changed = true;
                        ui.close_menu();
                    }
                    if ui.add_enabled(e.camera.is_none(), egui::Button::new("Camera")).clicked() {
                        e.camera = Some(CameraComp::default());
                        changed = true;
                        ui.close_menu();
                    }
                    if ui.add_enabled(e.light.is_none(), egui::Button::new("Directional Light")).clicked() {
                        e.light = Some(Light::default());
                        changed = true;
                        ui.close_menu();
                    }
                    if ui.add_enabled(e.script.is_empty(), egui::Button::new("Script")).clicked() {
                        e.script = "fn start() {\n}\n\nfn update(dt) {\n    this.ry += 45.0 * dt;\n}\n".into();
                        self.bottom_tab = BottomTab::Script;
                        changed = true;
                        ui.close_menu();
                    }
                });
                if ui.button(RichText::new("🗑 Delete Entity").color(Color32::LIGHT_RED)).clicked() {
                    remove = true;
                }
            });
        });

        if remove {
            self.delete_selected();
            return;
        }
        if changed {
            if !self.editing {
                // Snapshot the pre-edit state as one undo step per interaction.
                let cur = self.scene.get(id).cloned();
                if let Some(slot) = self.scene.get_mut(id) {
                    *slot = before;
                }
                self.push_undo();
                if let (Some(slot), Some(_)) = (self.scene.get_mut(id), cur) {
                    *slot = e.clone();
                }
                self.editing = true;
            }
            if e.name != self.scene.get(id).map(|x| x.name.clone()).unwrap_or_default() && self.scene.entities.iter().any(|x| x.id != id && x.name == e.name) {
                e.name = self.scene.unique_name(&e.name);
            }
            if let Some(slot) = self.scene.get_mut(id) {
                *slot = e;
            }
            self.dirty = true;
        }
        if !ui.ctx().input(|i| i.pointer.any_down()) && !ui.ctx().wants_keyboard_input() {
            self.editing = false;
        }
    }

    fn environment_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        let env = &mut self.scene.environment;
        let mut changed = false;
        section(ui, "Environment", |ui| {
            egui::Grid::new("env").num_columns(2).show(ui, |ui| {
                ui.label("Scene name");
                changed |= ui.text_edit_singleline(&mut self.scene.name).changed();
                ui.end_row();
                ui.label("Sky top");
                changed |= ui.color_edit_button_rgb(&mut env.sky_top).changed();
                ui.end_row();
                ui.label("Sky horizon");
                changed |= ui.color_edit_button_rgb(&mut env.sky_bottom).changed();
                ui.end_row();
                ui.label("Ambient");
                changed |= ui.add(egui::Slider::new(&mut env.ambient, 0.0..=1.0)).changed();
                ui.end_row();
                ui.label("Gravity");
                changed |= ui.add(egui::DragValue::new(&mut env.gravity).speed(0.1)).changed();
                ui.end_row();
            });
        });
        if changed {
            self.dirty = true;
        }
    }

    // ------------------------------------------------------------ UI: bottom panel

    fn bottom_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.bottom_tab, BottomTab::Console, "🖥 Console");
            ui.selectable_value(&mut self.bottom_tab, BottomTab::Script, "📜 Script");
            if self.bottom_tab == BottomTab::Console {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Clear").clicked() {
                        self.console.clear();
                    }
                });
            }
        });
        ui.separator();
        match self.bottom_tab {
            BottomTab::Console => {
                egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false; 2]).show(ui, |ui| {
                    for (lvl, msg) in &self.console {
                        let (icon, c) = match lvl {
                            LogLevel::Info => ("ℹ", theme::BONE),
                            LogLevel::Warn => ("⚠", theme::GOLD),
                            LogLevel::Error => ("⛔", Color32::LIGHT_RED),
                            LogLevel::Ai => ("✨", ACCENT),
                        };
                        ui.label(RichText::new(format!("{icon} {msg}")).monospace().size(11.5).color(c));
                    }
                });
            }
            BottomTab::Script => self.script_ui(ui),
        }
    }

    fn script_ui(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            ui.label(RichText::new("Select an entity to edit its script.").weak());
            return;
        };
        let name = self.scene.get(id).map(|e| e.name.clone()).unwrap_or_default();
        let mut code = self.scene.get(id).map(|e| e.script.clone()).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("📜 {name}")).strong());
            if ui.button("✔ Check").clicked() {
                match rhai::Engine::new().compile(&code) {
                    Ok(_) => self.log(LogLevel::Info, format!("{name}: script compiles")),
                    Err(e) => self.log(LogLevel::Error, format!("{name}: {e}")),
                }
                self.bottom_tab = BottomTab::Console;
            }
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut self.dialogs.script_goal).hint_text("✨ What should this object do?").desired_width(260.0));
            if ui.add_enabled(!self.chat.busy() && !self.dialogs.script_goal.trim().is_empty(), egui::Button::new("Write with AI")).clicked() {
                self.chat.input = format!(
                    "Write a Rhai script for the entity '{name}' using set_script or ask_coder. Goal: {}",
                    self.dialogs.script_goal.trim()
                );
                self.dialogs.script_goal.clear();
                self.chat.mode = crate::ai::agent::Mode::Agent;
                let ctx = ui.ctx().clone();
                self.send_chat(&ctx);
            }
        });
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let r = ui.add(
                egui::TextEdit::multiline(&mut code)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(12)
                    .hint_text("fn update(dt) {\n    this.ry += 90.0 * dt;\n}"),
            );
            if r.changed() {
                if !self.editing {
                    self.push_undo();
                    self.editing = true;
                }
                if let Some(e) = self.scene.get_mut(id) {
                    e.script = code;
                }
                self.dirty = true;
            }
            if r.lost_focus() {
                self.editing = false;
            }
        });
    }

    // ------------------------------------------------------------ UI: viewport

    fn viewport_ui(&mut self, ui: &mut egui::Ui) {
        if !self.player_mode {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.view_tab, ViewTab::Scene, "# Scene");
            ui.selectable_value(&mut self.view_tab, ViewTab::Game, "🎮 Game");
            ui.separator();
            ui.label(RichText::new("RMB orbit · MMB/Shift+RMB pan · wheel zoom · RMB+WASD fly · F focus").small().weak());
        });
        }
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let aspect = rect.width() / rect.height().max(1.0);
        let playing = self.runtime.is_some();
        let game_view = self.view_tab == ViewTab::Game;
        let scene: &Scene = match &self.runtime {
            Some(rt) => &rt.scene,
            None => &self.scene,
        };

        // Camera choice.
        let cam: CameraView = if game_view {
            render::main_camera(scene).and_then(|c| render::entity_camera(scene, c, aspect)).unwrap_or_else(|| self.cam.view(aspect))
        } else {
            self.cam.view(aspect)
        };

        // Editor camera controls (scene view, or game view without a camera).
        if !game_view {
            let input = ui.input(|i| i.clone());
            if resp.hovered() {
                let scroll = input.smooth_scroll_delta.y;
                if scroll != 0.0 {
                    self.cam.zoom(scroll);
                }
            }
            let shift = input.modifiers.shift;
            if resp.dragged_by(egui::PointerButton::Secondary) && !shift {
                let d = resp.drag_delta();
                self.cam.orbit(d.x, d.y);
            }
            if resp.dragged_by(egui::PointerButton::Middle) || (resp.dragged_by(egui::PointerButton::Secondary) && shift) {
                let d = resp.drag_delta();
                self.cam.pan(d.x, d.y);
            }
            if input.pointer.secondary_down() && resp.hovered() || resp.dragged_by(egui::PointerButton::Secondary) {
                let k = |key| if input.key_down(key) { 1.0 } else { 0.0 };
                let mv = Vec3::new(k(egui::Key::D) - k(egui::Key::A), k(egui::Key::E) - k(egui::Key::Q), k(egui::Key::W) - k(egui::Key::S));
                if mv != Vec3::ZERO {
                    self.cam.fly(mv * input.stable_dt.min(0.05) * 10.0);
                    ui.ctx().request_repaint();
                }
            }
            if resp.hovered() && input.key_pressed(egui::Key::F) && !ui.ctx().wants_keyboard_input() {
                if let Some(s) = self.selected {
                    self.focus(s);
                }
            }
        }

        // Gizmo (scene view, edit mode only).
        let mut gizmo_axes: Vec<(usize, egui::Pos2, egui::Pos2, f32)> = vec![];
        if !game_view && !playing {
            if let Some(sel) = self.selected.filter(|s| self.scene.get(*s).is_some()) {
                let p = self.scene.world_matrix(sel).w_axis.truncate();
                let len = (cam.eye.distance(p) * 0.14).max(0.2);
                if let Some(sp) = cam.project(p, rect) {
                    for (i, axis) in [Vec3::X, Vec3::Y, Vec3::Z].iter().enumerate() {
                        if let Some(ep) = cam.project(p + *axis * len, rect) {
                            gizmo_axes.push((i, sp, ep, len));
                        }
                    }
                }
            }
            let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
            if resp.drag_started_by(egui::PointerButton::Primary) {
                if let Some(mp) = pointer {
                    let hit = gizmo_axes.iter().map(|(i, a, b, _)| (*i, seg_dist(mp, *a, *b))).filter(|(_, d)| *d < 9.0).min_by(|a, b| a.1.total_cmp(&b.1));
                    if let (Some((axis, _)), Some(sel)) = (hit, self.selected) {
                        self.push_undo();
                        let start_pos = self.scene.get(sel).unwrap().transform.position;
                        self.gizmo = Some(GizmoDrag { axis, start_pos, accumulated: 0.0 });
                    }
                }
            }
            if let (Some(g), Some(sel)) = (&mut self.gizmo, self.selected) {
                if resp.dragged_by(egui::PointerButton::Primary) {
                    if let Some((_, a, b, len)) = gizmo_axes.iter().find(|(i, ..)| *i == g.axis) {
                        let sv = *b - *a;
                        let sl = sv.length().max(1.0);
                        let d = resp.drag_delta().dot(sv / sl);
                        g.accumulated += d / sl * len;
                        let mut np = g.start_pos;
                        let snap = ui.input(|i| i.modifiers.ctrl);
                        let mut amt = g.accumulated;
                        if snap {
                            amt = (amt * 2.0).round() / 2.0;
                        }
                        np[g.axis] += amt;
                        if let Some(e) = self.scene.get_mut(sel) {
                            e.transform.position = np;
                        }
                    }
                }
            }
            if resp.drag_stopped() {
                self.gizmo = None;
            }
            if resp.clicked() && self.gizmo.is_none() {
                if let Some(mp) = resp.interact_pointer_pos() {
                    let (o, d) = cam.ray(mp, rect);
                    self.selected = render::pick(&self.scene, o, d);
                }
            }
        }

        // Paint 3D.
        let scene: &Scene = match &self.runtime {
            Some(rt) => &rt.scene,
            None => &self.scene,
        };
        let sel = if game_view { None } else { self.selected };
        let dl = DrawList::from_scene(scene, cam, sel, !game_view);
        if let Some(r) = &self.renderer {
            let r = r.clone();
            let cb = egui_glow::CallbackFn::new(move |info, painter| {
                r.lock().unwrap().paint(painter.gl(), &dl, &info);
            });
            ui.painter().add(egui::PaintCallback { rect, callback: Arc::new(cb) });
        } else {
            ui.painter().rect_filled(rect, 0.0, theme::BG0);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "OpenGL 3.3 renderer unavailable", egui::FontId::proportional(16.0), Color32::LIGHT_RED);
        }

        let painter = ui.painter_at(rect);
        // Icons for lights / cameras in scene view.
        if !game_view {
            for e in &scene.entities {
                if e.mesh != MeshKind::None || !scene.is_active(e.id) {
                    continue;
                }
                let p = scene.world_matrix(e.id).w_axis.truncate();
                if let Some(sp) = cam.project(p, rect) {
                    let icon = if e.camera.is_some() { "🎥" } else if e.light.is_some() { "☀" } else { "◌" };
                    let c = if Some(e.id) == self.selected { ACCENT } else { theme::BONE };
                    painter.text(sp, egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(18.0), c);
                }
            }
            // Gizmo arrows.
            for (i, a, b, _) in &gizmo_axes {
                let c = [Color32::from_rgb(235, 75, 65), Color32::from_rgb(110, 210, 90), Color32::from_rgb(75, 140, 245)][*i];
                let active = self.gizmo.as_ref().map_or(false, |g| g.axis == *i);
                painter.line_segment([*a, *b], egui::Stroke::new(if active { 4.0_f32 } else { 2.5 }, c));
                painter.circle_filled(*b, if active { 6.5 } else { 5.0 }, c);
            }
        }
        // HUD.
        if let Some(rt) = &self.runtime {
            let hud = rt.hud();
            if !hud.is_empty() {
                let g = painter.layout_no_wrap(hud, egui::FontId::proportional(20.0), Color32::WHITE);
                let pos = rect.left_top() + egui::vec2(16.0, 14.0);
                painter.rect_filled(egui::Rect::from_min_size(pos - egui::vec2(8.0, 4.0), g.size() + egui::vec2(16.0, 8.0)), 6.0, Color32::from_black_alpha(140));
                painter.galley(pos, g, Color32::WHITE);
            }
            painter.rect_stroke(rect.shrink(1.0), 0.0, egui::Stroke::new(2.0_f32, if rt.paused { theme::GOLD } else { ACCENT }), egui::StrokeKind::Inside);
            if game_view {
                resp.request_focus();
            }
        }
        if game_view && render::main_camera(scene).is_none() {
            painter.text(rect.center_bottom() - egui::vec2(0.0, 18.0), egui::Align2::CENTER_CENTER, "No camera in scene — showing editor view", egui::FontId::proportional(13.0), theme::GOLD);
        }
    }

    // ------------------------------------------------------------ dialogs

    fn dialogs_ui(&mut self, ctx: &egui::Context) {
        if let Some(mut path) = self.dialogs.open.take() {
            let mut keep = true;
            egui::Window::new("Open Scene").collapsible(false).show(ctx, |ui| {
                ui.label("Scene file (.json)");
                ui.add(egui::TextEdit::singleline(&mut path).desired_width(460.0));
                let found = find_scenes(&self.settings.projects_dir);
                if !found.is_empty() {
                    ui.label(RichText::new("Projects").strong());
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        for f in found {
                            if ui.selectable_label(path == f.display().to_string(), f.display().to_string()).clicked() {
                                path = f.display().to_string();
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        self.open_scene(PathBuf::from(path.trim()));
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
            if keep {
                self.dialogs.open = Some(path);
            }
        }
        if let Some(mut path) = self.dialogs.save_as.take() {
            let mut keep = true;
            egui::Window::new("Save Scene As").collapsible(false).show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut path).desired_width(460.0));
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        let mut p = PathBuf::from(path.trim());
                        if p.extension().is_none() {
                            p.set_extension("json");
                        }
                        self.save_to(p);
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
            if keep {
                self.dialogs.save_as = Some(path);
            }
        }
        if let Some(mut path) = self.dialogs.build.take() {
            let mut keep = true;
            egui::Window::new("Build Game").collapsible(false).show(ctx, |ui| {
                ui.label("Output folder — Forge copies its player runtime and the scene there.");
                ui.add(egui::TextEdit::singleline(&mut path).desired_width(460.0));
                ui.horizontal(|ui| {
                    if ui.button("🔨 Build").clicked() {
                        self.build_game(PathBuf::from(path.trim()));
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
            if keep {
                self.dialogs.build = Some(path);
            }
        }
        if self.dialogs.settings {
            let mut open = true;
            egui::Window::new("AI Settings").open(&mut open).default_width(520.0).show(ctx, |ui| self.settings_ui(ui));
            self.dialogs.settings = open;
        }
        if self.dialogs.scripting_help {
            let mut open = true;
            egui::Window::new("Scripting reference").open(&mut open).default_width(560.0).show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                    ui.label(RichText::new(tools::SCRIPT_DOCS).monospace().size(12.0));
                });
            });
            self.dialogs.scripting_help = open;
        }
        if self.dialogs.about {
            let mut open = true;
            egui::Window::new("About Forge").open(&mut open).collapsible(false).show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(96.0, 96.0), egui::Sense::hover());
                    theme::draw_logo(ui.painter(), r);
                    ui.label(RichText::new("Forge").size(24.0).strong());
                    ui.label(format!("v{} — AI-native game engine in Rust", env!("CARGO_PKG_VERSION")));
                    ui.small("egui · OpenGL · Rhai · llama.cpp · OpenRouter");
                });
            });
            self.dialogs.about = open;
        }
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.settings;
        ui.label(RichText::new("API keys").strong());
        egui::Grid::new("keys").num_columns(2).show(ui, |ui| {
            ui.label("OpenRouter");
            ui.add(egui::TextEdit::singleline(&mut self.secrets.openrouter_key).password(true).desired_width(340.0));
            ui.end_row();
            ui.label("Hugging Face (opt.)");
            ui.add(egui::TextEdit::singleline(&mut self.secrets.hf_token).password(true).desired_width(340.0));
            ui.end_row();
        });
        ui.horizontal(|ui| {
            if ui.button("💾 Save keys").clicked() {
                let msg = match self.secrets.save() {
                    Ok(()) => format!("Keys saved to {}", crate::config::env_file().display()),
                    Err(e) => format!("Could not save keys: {e}"),
                };
                self.console.push((LogLevel::Info, msg));
            }
            ui.small(crate::config::env_file().display().to_string());
        });
        ui.separator();
        ui.label(RichText::new("Agents").strong());
        let mut options: Vec<(String, String)> = vec![(String::new(), "(same as chat model)".into())];
        options.extend(crate::ai::models::CLOUD_MODELS.iter().map(|m| (m.id.to_string(), m.name.to_string())));
        options.extend(s.custom_models.iter().map(|m| (m.clone(), m.clone())));
        options.extend(self.library.installed_models().into_iter().map(|m| (m.clone(), crate::ai::models::display_name(&m))));
        options.extend(self.library.ollama_models().into_iter().map(|m| (m.clone(), crate::ai::models::display_name(&m))));
        egui::Grid::new("agents").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            for (label, value) in [
                ("🧭 Router", &mut s.router_model),
                ("🗺 Planner", &mut s.planner_model),
                ("🔨 Builder", &mut s.builder_model),
                ("⌨ Coder", &mut s.coder_model),
                ("🔍 Reviewer", &mut s.reviewer_model),
            ] {
                ui.label(label);
                let shown = options.iter().find(|(id, _)| id == value).map(|(_, n)| n.clone()).unwrap_or_else(|| value.clone());
                egui::ComboBox::from_id_salt(label).selected_text(shown).width(300.0).show_ui(ui, |ui| {
                    for (id, n) in &options {
                        let n = if label.contains("Router") && id.is_empty() { "(fast heuristic)" } else { n.as_str() };
                        ui.selectable_value(value, id.clone(), n);
                    }
                });
                ui.end_row();
            }
        });
        ui.checkbox(&mut s.use_planner, "Planner agent (plans before building)");
        ui.checkbox(&mut s.use_reviewer, "Reviewer agent (checks result, one fix round)");
        ui.horizontal(|ui| {
            ui.label("Max builder steps");
            ui.add(egui::DragValue::new(&mut s.max_agent_steps).range(1..=30));
            ui.label("Temperature");
            ui.add(egui::Slider::new(&mut s.temperature, 0.0..=1.5));
        });
        ui.small("Tip: free OpenRouter models are rate limited; turning off Planner/Reviewer uses fewer requests.");
        if ui.button("Save settings").clicked() {
            s.save();
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(true).show(ui, body);
}

fn section_removable(ui: &mut egui::Ui, title: &str, keep: &mut bool, body: impl FnOnce(&mut egui::Ui)) {
    let id = ui.make_persistent_id(title);
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            ui.label(RichText::new(title).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✖").on_hover_text("Remove component").clicked() {
                    *keep = false;
                }
            });
        })
        .body(body);
}

fn seg_dist(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

fn find_scenes(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Ok(rd2) = std::fs::read_dir(&p) {
                    out.extend(rd2.flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().ends_with(".json")));
                }
            }
        }
    }
    out.sort();
    out
}

impl eframe::App for ForgeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        // Setup wizard takes over the window.
        if let Some(w) = &mut self.wizard {
            if let Some(res) = w.ui(ctx, &mut self.settings, &mut self.secrets, &mut self.library) {
                self.wizard = None;
                self.library.refresh_installed(&self.settings);
                if res.scene_path.is_file() {
                    self.open_scene(res.scene_path);
                } else {
                    self.scene = res.template.build();
                    self.scene.name = res
                        .scene_path
                        .parent()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Main".into());
                    self.save_to(res.scene_path);
                }
                self.log(LogLevel::Info, "Setup complete — welcome to Forge!".into());
            }
            return;
        }

        // Standalone player (exported game).
        if self.player_mode {
            self.tick_runtime(ctx, dt);
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                self.view_tab = ViewTab::Game;
                self.viewport_ui(ui);
            });
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }

        // Agent events → execute tools on the scene.
        let mut chat = std::mem::take(&mut self.chat);
        chat.poll(|call| self.exec_tool(call));
        self.chat = chat;

        self.tick_runtime(ctx, dt);

        // Shortcuts.
        let typing = ctx.wants_keyboard_input();
        let (save, undo, redo, dup, del, play) = ctx.input(|i| {
            let c = i.modifiers.command;
            (
                c && i.key_pressed(egui::Key::S),
                c && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                c && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
                c && i.key_pressed(egui::Key::D),
                i.key_pressed(egui::Key::Delete),
                c && i.key_pressed(egui::Key::P),
            )
        });
        if save {
            self.save();
        }
        if play {
            if self.runtime.is_some() { self.stop() } else { self.play() }
        }
        if !typing && self.runtime.is_none() {
            if undo { self.undo(); }
            if redo { self.redo(); }
            if dup { self.duplicate_selected(); }
            if del { self.delete_selected(); }
        }

        // Window title.
        let title = format!(
            "Forge — {}{}",
            self.scene_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| self.scene.name.clone()),
            if self.dirty { " *" } else { "" }
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        egui::TopBottomPanel::top("menu").frame(egui::Frame::new().fill(theme::BG0).inner_margin(egui::Margin::symmetric(8, 4))).show(ctx, |ui| {
            self.menu_bar(ui, ctx);
        });

        // Right: Forge AI chat.
        egui::SidePanel::right("chat").default_width(360.0).width_range(280.0..=640.0).show(ctx, |ui| {
            let mut local = self.library.installed_models();
            local.extend(self.library.ollama_models());
            let current = self.settings.selected_model.clone();
            let reqs = self.chat.ui(ui, &current, &local);
            for r in reqs {
                match r {
                    ChatRequest::Send => self.send_chat(ctx),
                    ChatRequest::SelectModel(m) => self.select_model(m, ctx),
                    ChatRequest::Settings => self.dialogs.settings = true,
                }
            }
        });

        // Left: hierarchy / library.
        egui::SidePanel::left("left").default_width(270.0).width_range(200.0..=480.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.left_tab, LeftTab::Hierarchy, "☰ Hierarchy");
                ui.selectable_value(&mut self.left_tab, LeftTab::Library, "📚 Library");
            });
            ui.separator();
            match self.left_tab {
                LeftTab::Hierarchy => self.hierarchy_ui(ui),
                LeftTab::Library => {
                    let current = self.settings.selected_model.clone();
                    let actions = self.library.ui(ui, &mut self.settings, &self.secrets, &current);
                    for a in actions {
                        match a {
                            LibAction::UseModel(m) => self.select_model(m, ctx),
                            LibAction::Log(m) => self.log(LogLevel::Info, m),
                        }
                    }
                }
            }
        });

        // Inspector (right of viewport).
        egui::SidePanel::right("inspector").default_width(290.0).width_range(220.0..=480.0).show(ctx, |ui| {
            ui.label(RichText::new("🔎 Inspector").strong());
            ui.separator();
            if self.runtime.is_some() {
                ui.label(RichText::new("Playing — edits are disabled. Stop (■) to edit.").color(theme::GOLD));
            } else {
                self.inspector_ui(ui);
            }
        });

        egui::TopBottomPanel::bottom("bottom").resizable(true).default_height(190.0).height_range(90.0..=500.0).show(ctx, |ui| {
            self.bottom_ui(ui);
        });

        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG0).inner_margin(4)).show(ctx, |ui| {
            self.viewport_ui(ui);
        });

        self.dialogs_ui(ctx);
        self.frame_ms = self.frame_ms * 0.9 + now.elapsed().as_secs_f32() * 1000.0 * 0.1;
        if self.chat.busy() || self.library.downloads.iter().any(|_| true) {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.settings.save();
        self.library.stop_llama();
    }
}
