//! Play mode: Rhai scripting + lightweight AABB physics.
//!
//! Scripts receive the entity as `this` (an object map):
//! ```rhai
//! fn start() { this.speed = 6.0; }
//! fn update(dt) {
//!     if key("A") { this.x -= this.speed * dt; }
//!     if key("SPACE") && this.grounded { this.vy = 8.0; }
//!     if touching(this.name, "Coin") { destroy_touching(this.name, "Coin"); set_global("score", get_global("score") + 1); }
//!     hud("Score: " + get_global("score"));
//! }
//! ```

use crate::scene::{parse_color, color_hex, EntityId, MeshKind, Scene};
use glam::Vec3;
use rhai::{Dynamic, Engine, Map, Scope, AST};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Default)]
struct Body {
    id: EntityId,
    tag: String,
    lo: Vec3,
    hi: Vec3,
    pos: Vec3,
}

enum Cmd {
    Destroy(String),
    Spawn { name: String, mesh: MeshKind, pos: Vec3, color: Option<[f32; 3]> },
    SetColor(String, [f32; 3]),
}

#[derive(Default)]
struct Shared {
    bodies: HashMap<String, Body>,
    touching: HashSet<(String, String)>,
    keys_down: HashSet<String>,
    keys_pressed: HashSet<String>,
    commands: Vec<Cmd>,
    hud: String,
    logs: Vec<String>,
    globals: Map,
    rng: u64,
}

impl Shared {
    fn touching_names(&self, a: &str, b: &str) -> Vec<String> {
        let mut out = vec![];
        for (x, y) in &self.touching {
            let other = if x == a { y } else if y == a { x } else { continue };
            let tag_match = self.bodies.get(other).map_or(false, |o| o.tag.eq_ignore_ascii_case(b));
            if other.starts_with(b) || tag_match {
                out.push(other.clone());
            }
        }
        out
    }
}

struct ScriptInst {
    ast: AST,
    this: Dynamic,
    started: bool,
    has_update: bool,
    has_start: bool,
}

pub struct Runtime {
    pub scene: Scene,
    engine: Engine,
    scripts: HashMap<EntityId, ScriptInst>,
    shared: Rc<RefCell<Shared>>,
    cam_offsets: HashMap<EntityId, Vec3>,
    pub time: f32,
    pub paused: bool,
}

fn normalize_key(k: &str) -> String {
    let k = k.trim().to_ascii_uppercase().replace(' ', "");
    match k.as_str() {
        "LEFT" => "ARROWLEFT".into(),
        "RIGHT" => "ARROWRIGHT".into(),
        "UP" => "ARROWUP".into(),
        "DOWN" => "ARROWDOWN".into(),
        "RETURN" => "ENTER".into(),
        "ESC" => "ESCAPE".into(),
        _ => k,
    }
}

fn dyn_f32(d: Option<&Dynamic>) -> Option<f32> {
    let d = d?;
    d.as_float().ok().or_else(|| d.as_int().ok().map(|i| i as f32))
}

impl Runtime {
    pub fn new(scene: Scene) -> (Self, Vec<String>) {
        let shared = Rc::new(RefCell::new(Shared { rng: 0x9E3779B97F4A7C15, ..Default::default() }));
        let mut engine = Engine::new();
        engine.set_max_operations(2_000_000);
        engine.set_max_call_levels(48);
        {
            let sh = shared.clone();
            engine.on_print(move |s| sh.borrow_mut().logs.push(s.to_string()));
            let sh = shared.clone();
            engine.on_debug(move |s, _, _| sh.borrow_mut().logs.push(s.to_string()));
        }
        let sh = shared.clone();
        engine.register_fn("log", move |s: &str| sh.borrow_mut().logs.push(s.to_string()));
        let sh = shared.clone();
        engine.register_fn("key", move |k: &str| sh.borrow().keys_down.contains(&normalize_key(k)));
        let sh = shared.clone();
        engine.register_fn("key_pressed", move |k: &str| sh.borrow().keys_pressed.contains(&normalize_key(k)));
        let sh = shared.clone();
        engine.register_fn("exists", move |n: &str| sh.borrow().bodies.contains_key(n));
        let sh = shared.clone();
        engine.register_fn("pos_of", move |n: &str| -> rhai::Array {
            let p = sh.borrow().bodies.get(n).map(|b| b.pos).unwrap_or(Vec3::ZERO);
            vec![Dynamic::from_float(p.x), Dynamic::from_float(p.y), Dynamic::from_float(p.z)]
        });
        let sh = shared.clone();
        engine.register_fn("distance", move |a: &str, b: &str| -> f32 {
            let s = sh.borrow();
            match (s.bodies.get(a), s.bodies.get(b)) {
                (Some(x), Some(y)) => x.pos.distance(y.pos),
                _ => f32::MAX,
            }
        });
        let sh = shared.clone();
        engine.register_fn("touching", move |a: &str, b: &str| !sh.borrow().touching_names(a, b).is_empty());
        let sh = shared.clone();
        engine.register_fn("destroy_touching", move |a: &str, b: &str| -> i64 {
            let mut s = sh.borrow_mut();
            let names = s.touching_names(a, b);
            let n = names.len() as i64;
            for name in names {
                s.touching.retain(|(x, y)| x != &name && y != &name);
                s.bodies.remove(&name);
                s.commands.push(Cmd::Destroy(name));
            }
            n
        });
        let sh = shared.clone();
        engine.register_fn("destroy", move |n: &str| {
            let mut s = sh.borrow_mut();
            s.bodies.remove(n);
            s.commands.push(Cmd::Destroy(n.to_string()));
        });
        let sh = shared.clone();
        engine.register_fn("spawn", move |name: &str, mesh: &str, x: f32, y: f32, z: f32, color: &str| {
            sh.borrow_mut().commands.push(Cmd::Spawn {
                name: name.into(),
                mesh: MeshKind::parse(mesh),
                pos: Vec3::new(x, y, z),
                color: parse_color(&serde_json::Value::String(color.into())),
            });
        });
        let sh = shared.clone();
        engine.register_fn("set_color", move |n: &str, c: &str| {
            if let Some(c) = parse_color(&serde_json::Value::String(c.into())) {
                sh.borrow_mut().commands.push(Cmd::SetColor(n.into(), c));
            }
        });
        let sh = shared.clone();
        engine.register_fn("hud", move |t: &str| sh.borrow_mut().hud = t.to_string());
        let sh = shared.clone();
        engine.register_fn("random", move |lo: f32, hi: f32| -> f32 {
            let mut s = sh.borrow_mut();
            s.rng ^= s.rng << 13;
            s.rng ^= s.rng >> 7;
            s.rng ^= s.rng << 17;
            lo + (s.rng >> 40) as f32 / (1u64 << 24) as f32 * (hi - lo)
        });
        let sh = shared.clone();
        engine.register_fn("get_global", move |k: &str| -> Dynamic {
            sh.borrow().globals.get(k).cloned().unwrap_or(Dynamic::from_int(0))
        });
        let sh = shared.clone();
        engine.register_fn("set_global", move |k: &str, v: Dynamic| {
            sh.borrow_mut().globals.insert(k.into(), v);
        });

        let mut rt = Self {
            scene,
            engine,
            scripts: HashMap::new(),
            shared,
            cam_offsets: HashMap::new(),
            time: 0.0,
            paused: false,
        };
        let errors = rt.compile_all();
        rt.init_cameras();
        (rt, errors)
    }

    fn compile_all(&mut self) -> Vec<String> {
        let mut errors = vec![];
        for e in &self.scene.entities {
            if e.script.trim().is_empty() {
                continue;
            }
            match self.engine.compile(&e.script) {
                Ok(ast) => {
                    let has = |n: &str| ast.iter_functions().any(|f| f.name == n);
                    let (has_update, has_start) = (has("update"), has("start"));
                    // Run top-level statements once (e.g. constants / print).
                    let mut scope = Scope::new();
                    if let Err(err) = self.engine.run_ast_with_scope(&mut scope, &ast) {
                        errors.push(format!("{}: {err}", e.name));
                    }
                    let mut m = Map::new();
                    m.insert("name".into(), e.name.clone().into());
                    m.insert("tag".into(), e.tag.clone().into());
                    m.insert("id".into(), Dynamic::from_int(e.id as i64));
                    self.scripts.insert(e.id, ScriptInst { ast, this: Dynamic::from_map(m), started: false, has_update, has_start });
                }
                Err(err) => errors.push(format!("{} (script): {err}", e.name)),
            }
        }
        errors
    }

    fn init_cameras(&mut self) {
        for e in &self.scene.entities {
            if let Some(c) = &e.camera {
                if let Some(t) = self.scene.find(&c.follow) {
                    let tp = self.scene.get(t).unwrap().transform.position;
                    self.cam_offsets.insert(e.id, e.transform.position - tp);
                }
            }
        }
    }

    pub fn set_input(&mut self, down: HashSet<String>, pressed: HashSet<String>) {
        let mut s = self.shared.borrow_mut();
        s.keys_down = down;
        s.keys_pressed = pressed;
    }

    pub fn hud(&self) -> String {
        self.shared.borrow().hud.clone()
    }

    pub fn take_logs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.shared.borrow_mut().logs)
    }

    fn world_aabb(&self, id: EntityId) -> (Vec3, Vec3) {
        let e = self.scene.get(id).unwrap();
        let m = self.scene.world_matrix(id);
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for i in 0..8 {
            let mut p = Vec3::new(
                if i & 1 == 0 { -0.5 } else { 0.5 },
                if i & 2 == 0 { -0.5 } else { 0.5 },
                if i & 4 == 0 { -0.5 } else { 0.5 },
            );
            if e.mesh == MeshKind::Plane {
                p.y = 0.0;
            }
            let w = m.transform_point3(p);
            lo = lo.min(w);
            hi = hi.max(w);
        }
        if e.mesh == MeshKind::Plane {
            lo.y -= 0.25;
        }
        (lo, hi)
    }

    fn refresh_bodies(&mut self) {
        let mut bodies = HashMap::new();
        for e in &self.scene.entities {
            if !self.scene.is_active(e.id) {
                continue;
            }
            let (lo, hi) = if e.mesh != MeshKind::None {
                self.world_aabb(e.id)
            } else {
                (e.transform.position, e.transform.position)
            };
            bodies.insert(e.name.clone(), Body { id: e.id, tag: e.tag.clone(), lo, hi, pos: e.transform.position });
        }
        // Overlap pairs (only between collidable meshes).
        let mut touching = HashSet::new();
        let list: Vec<(&String, &Body)> = bodies
            .iter()
            .filter(|(_, b)| {
                let e = self.scene.get(b.id).unwrap();
                e.mesh != MeshKind::None && (e.collider || e.trigger)
            })
            .collect();
        let eps = 0.02;
        for i in 0..list.len() {
            for j in i + 1..list.len() {
                let (a, b) = (list[i].1, list[j].1);
                if a.lo.x <= b.hi.x + eps && a.hi.x + eps >= b.lo.x
                    && a.lo.y <= b.hi.y + eps && a.hi.y + eps >= b.lo.y
                    && a.lo.z <= b.hi.z + eps && a.hi.z + eps >= b.lo.z
                {
                    touching.insert((list[i].0.clone(), list[j].0.clone()));
                }
            }
        }
        let mut s = self.shared.borrow_mut();
        s.bodies = bodies;
        s.touching = touching;
    }

    pub fn step(&mut self, dt: f32) -> Vec<String> {
        let mut errors = vec![];
        if self.paused {
            return errors;
        }
        let dt = dt.min(1.0 / 20.0);
        self.time += dt;
        self.physics(dt);
        self.refresh_bodies();

        // Scripts.
        let ids: Vec<EntityId> = self.scripts.keys().copied().collect();
        for id in ids {
            if !self.scene.get(id).map_or(false, |_| self.scene.is_active(id)) {
                continue;
            }
            self.write_this(id);
            let inst = self.scripts.get_mut(&id).unwrap();
            let mut scope = Scope::new();
            let call = |name: &str, args: Vec<Dynamic>, inst: &mut ScriptInst, engine: &Engine, scope: &mut Scope| {
                let opts = rhai::CallFnOptions::new().eval_ast(false).rewind_scope(true).bind_this_ptr(&mut inst.this);
                engine.call_fn_with_options::<Dynamic>(opts, scope, &inst.ast, name, args).map(|_| ())
            };
            if !inst.started {
                inst.started = true;
                if inst.has_start {
                    if let Err(e) = call("start", vec![], inst, &self.engine, &mut scope) {
                        errors.push(format!("start(): {e}"));
                    }
                }
            }
            if inst.has_update {
                if let Err(e) = call("update", vec![Dynamic::from_float(dt)], inst, &self.engine, &mut scope) {
                    let name = self.scene.get(id).map(|e| e.name.clone()).unwrap_or_default();
                    errors.push(format!("{name}.update(): {e}"));
                    self.scripts.get_mut(&id).unwrap().has_update = false; // don't spam
                }
            }
            self.read_this(id);
        }
        self.apply_commands();
        self.follow_cameras(dt);
        errors
    }

    fn write_this(&mut self, id: EntityId) {
        let Some(e) = self.scene.get(id) else { return };
        let t = &e.transform;
        let rb = e.rigidbody.as_ref();
        let fields: [(&str, Dynamic); 15] = [
            ("x", t.position.x.into()), ("y", t.position.y.into()), ("z", t.position.z.into()),
            ("rx", t.rotation.x.into()), ("ry", t.rotation.y.into()), ("rz", t.rotation.z.into()),
            ("sx", t.scale.x.into()), ("sy", t.scale.y.into()), ("sz", t.scale.z.into()),
            ("vx", rb.map_or(0.0, |r| r.velocity.x).into()),
            ("vy", rb.map_or(0.0, |r| r.velocity.y).into()),
            ("vz", rb.map_or(0.0, |r| r.velocity.z).into()),
            ("grounded", rb.map_or(false, |r| r.grounded).into()),
            ("color", color_hex(e.color).into()),
            ("time", self.time.into()),
        ];
        let active = e.active;
        if let Some(inst) = self.scripts.get_mut(&id) {
            if let Some(mut m) = inst.this.write_lock::<Map>() {
                for (k, v) in fields {
                    m.insert(k.into(), v);
                }
                m.insert("active".into(), active.into());
            }
        }
    }

    fn read_this(&mut self, id: EntityId) {
        let Some(inst) = self.scripts.get(&id) else { return };
        let Some(m) = inst.this.read_lock::<Map>().map(|m| m.clone()) else { return };
        let Some(e) = self.scene.get_mut(id) else { return };
        let g = |k: &str| dyn_f32(m.get(k));
        let t = &mut e.transform;
        t.position = Vec3::new(g("x").unwrap_or(t.position.x), g("y").unwrap_or(t.position.y), g("z").unwrap_or(t.position.z));
        t.rotation = Vec3::new(g("rx").unwrap_or(t.rotation.x), g("ry").unwrap_or(t.rotation.y), g("rz").unwrap_or(t.rotation.z));
        t.scale = Vec3::new(g("sx").unwrap_or(t.scale.x), g("sy").unwrap_or(t.scale.y), g("sz").unwrap_or(t.scale.z));
        if let Some(rb) = &mut e.rigidbody {
            rb.velocity = Vec3::new(g("vx").unwrap_or(0.0), g("vy").unwrap_or(0.0), g("vz").unwrap_or(0.0));
        }
        if let Some(c) = m.get("color").and_then(|c| c.clone().into_string().ok()) {
            if let Some(c) = parse_color(&serde_json::Value::String(c)) {
                e.color = c;
            }
        }
        if let Some(a) = m.get("active").and_then(|a| a.as_bool().ok()) {
            e.active = a;
        }
    }

    fn apply_commands(&mut self) {
        let cmds = std::mem::take(&mut self.shared.borrow_mut().commands);
        for c in cmds {
            match c {
                Cmd::Destroy(n) => {
                    if let Some(id) = self.scene.find(&n) {
                        self.scene.remove(id);
                        self.scripts.remove(&id);
                    }
                }
                Cmd::Spawn { name, mesh, pos, color } => {
                    let id = self.scene.spawn(&name, mesh);
                    let e = self.scene.get_mut(id).unwrap();
                    e.transform.position = pos;
                    if let Some(c) = color {
                        e.color = c;
                    }
                }
                Cmd::SetColor(n, c) => {
                    if let Some(id) = self.scene.find(&n) {
                        self.scene.get_mut(id).unwrap().color = c;
                    }
                }
            }
        }
    }

    fn follow_cameras(&mut self, dt: f32) {
        for (cam, off) in self.cam_offsets.clone() {
            let follow = self.scene.get(cam).and_then(|e| e.camera.as_ref()).map(|c| c.follow.clone()).unwrap_or_default();
            let Some(t) = self.scene.find(&follow) else { continue };
            let goal = self.scene.get(t).unwrap().transform.position + off;
            if let Some(e) = self.scene.get_mut(cam) {
                let k = 1.0 - (-8.0 * dt).exp();
                e.transform.position = e.transform.position.lerp(goal, k);
            }
        }
    }

    fn physics(&mut self, dt: f32) {
        let g = self.scene.environment.gravity;
        let statics: Vec<(EntityId, Vec3, Vec3)> = self
            .scene
            .entities
            .iter()
            .filter(|e| {
                e.collider && !e.trigger && e.mesh != MeshKind::None && self.scene.is_active(e.id)
                    && e.rigidbody.as_ref().map_or(true, |r| r.is_static)
            })
            .map(|e| {
                let (lo, hi) = self.world_aabb(e.id);
                (e.id, lo, hi)
            })
            .collect();
        let dynamic: Vec<EntityId> = self
            .scene
            .entities
            .iter()
            .filter(|e| e.rigidbody.as_ref().map_or(false, |r| !r.is_static) && self.scene.is_active(e.id))
            .map(|e| e.id)
            .collect();
        for id in dynamic {
            let (lo0, hi0) = self.world_aabb(id);
            let e = self.scene.get_mut(id).unwrap();
            let collider = e.collider && !e.trigger;
            let rb = e.rigidbody.as_mut().unwrap();
            if rb.use_gravity {
                rb.velocity.y += g * dt;
            }
            rb.velocity.y = rb.velocity.y.max(-60.0);
            let delta = rb.velocity * dt;
            e.transform.position += delta;
            let mut lo = lo0 + delta;
            let mut hi = hi0 + delta;
            let mut grounded = false;
            if collider {
                for (sid, slo, shi) in &statics {
                    if *sid == id {
                        continue;
                    }
                    let ox = hi.x.min(shi.x) - lo.x.max(slo.x);
                    let oy = hi.y.min(shi.y) - lo.y.max(slo.y);
                    let oz = hi.z.min(shi.z) - lo.z.max(slo.z);
                    if ox <= 0.0 || oy <= 0.0 || oz <= 0.0 {
                        continue;
                    }
                    let c = (lo + hi) * 0.5;
                    let sc = (*slo + *shi) * 0.5;
                    let mut push = Vec3::ZERO;
                    if oy <= ox && oy <= oz {
                        push.y = if c.y > sc.y { oy } else { -oy };
                    } else if ox <= oz {
                        push.x = if c.x > sc.x { ox } else { -ox };
                    } else {
                        push.z = if c.z > sc.z { oz } else { -oz };
                    }
                    e.transform.position += push;
                    lo += push;
                    hi += push;
                    let rb = e.rigidbody.as_mut().unwrap();
                    let b = rb.bounciness;
                    if push.y > 0.0 {
                        grounded = true;
                        if rb.velocity.y < 0.0 {
                            rb.velocity.y = -rb.velocity.y * b;
                        }
                        rb.velocity.x *= 1.0 - (8.0 * dt).min(1.0) * 0.15;
                        rb.velocity.z *= 1.0 - (8.0 * dt).min(1.0) * 0.15;
                    } else if push.y < 0.0 && rb.velocity.y > 0.0 {
                        rb.velocity.y = 0.0;
                    }
                    if push.x != 0.0 { rb.velocity.x = -rb.velocity.x * b; }
                    if push.z != 0.0 { rb.velocity.z = -rb.velocity.z * b; }
                }
            }
            e.rigidbody.as_mut().unwrap().grounded = grounded;
        }
    }
}

/// Collect egui key state into script-friendly names.
pub fn collect_input(ctx: &egui::Context) -> (HashSet<String>, HashSet<String>) {
    ctx.input(|i| {
        let down = i.keys_down.iter().map(|k| normalize_key(k.name())).collect();
        let pressed = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, repeat: false, .. } => Some(normalize_key(key.name())),
                _ => None,
            })
            .collect();
        (down, pressed)
    })
}
