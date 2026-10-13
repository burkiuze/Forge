//! Scene tools exposed to agents, a model-agnostic text tool-call protocol, and the executor.

use crate::scene::{parse_color, EntityId, Light, MeshKind, Pattern, Rigidbody, Scene};
use glam::Vec3;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const TOOL_DOCS: &str = r##"## Tools
Call tools by writing fenced blocks exactly like this (one JSON object per block, you may emit several blocks):
```tool
{"tool": "create_entity", "args": {"name": "Player", "mesh": "capsule", "position": [0, 1, 0], "color": "#f05a1e"}}
```
After your tool blocks STOP and wait: the engine runs them and replies with results. When the work is complete, reply with a short summary and NO tool blocks.

Available tools:
- get_scene {} -> current scene as JSON.
- create_entity {name, mesh: "cube|sphere|plane|cylinder|cone|capsule|none", position:[x,y,z], rotation:[x,y,z] (degrees), scale:[x,y,z], color:"#rrggbb", parent:"name", tag:"str", layer:"Default|Environment|Player|Enemy|Pickup|UI", rigidbody:{mass, gravity:bool, static:bool, bounciness}, collider:bool, trigger:bool, light:"directional|point" (+ light_color, intensity, range), material:{metallic:0..1, roughness:0..1, emission:"#hex", emission_strength:0..5, pattern:"none|checker|grid", tiling, texture:"Textures/file.png", unlit:bool}, script:"rhai code"}
- create_many {entities:[ ...create_entity args... ]}  ← prefer this for levels.
- update_entity {target:"name or id", ...any create_entity field}
- delete_entity {target}
- duplicate_entity {target, count:int, offset:[x,y,z]} -> copies spaced by offset.
- set_script {target, code} -> attach a Rhai script.
- ask_coder {target, goal} -> delegate script writing to the Coder agent (it writes & attaches the script).
- add_camera {name, position, rotation, fov, follow:"entity name"} (follow keeps the offset to the target during play).
- set_environment {sky_top:"#hex", sky_bottom:"#hex", ambient:0..1, gravity:-18}
- clear_scene {keep:["names to keep"]}
- select {target}
- web_search {query} -> top web results (title, url, snippet), searched from this device.
- fetch_url {url} -> readable text of a web page.
- memory_search {query} -> relevant long-term memories (user preferences, past projects).
- remember {fact} -> store a durable fact/preference for future sessions.
- todo_write {todos:[{content, status:"pending|in_progress|completed"}]} -> keep a visible task list for multi-step work (update it as you go).
- task {agent:"explore"|"coder", prompt} -> run a subagent (explore = read-only research; coder = writes scripts) and get its report.
- list_files {path:"Scripts"} · read_file {path} · write_file {path, content} · grep {pattern} -> project files (paths relative to the project folder).
- set_script also accepts {target, file:"Scripts/player.rhai"} to attach a script file.

World: +Y is up, +X is right, -Z is forward (away from the default camera). Units ≈ meters. A unit cube is 1×1×1 centered on its position, so a cube resting on the ground at y=0 has position.y = scale.y/2. Planes are 1×1 (scale them, e.g. [30,1,30]).
"##;

pub const SCRIPT_DOCS: &str = r##"## Scripting (Rhai)
Each entity may have a Rhai script. The engine calls `fn start()` once and `fn update(dt)` every frame. Inside them `this` is the entity:
  this.x this.y this.z  (position)   this.rx this.ry this.rz (rotation, degrees)   this.sx this.sy this.sz (scale)
  this.vx this.vy this.vz (velocity, needs rigidbody)   this.grounded (bool)   this.name   this.color ("#hex")   this.active   this.time
  Custom fields persist: `this.speed = 5.0;`
Built-in functions:
  key("W") / key("SPACE") / key("LEFT")  -> held;   key_pressed("SPACE") -> pressed this frame
  touching(this.name, "Coin")  -> overlapping an entity whose name starts with / tag equals "Coin"
  destroy_touching(this.name, "Coin") -> destroys those, returns count
  destroy("Name")   spawn("Name", "cube", x, y, z, "#ff0000")   set_color("Name", "#00ff00")
  pos_of("Name") -> [x,y,z]   distance("A", "B")   exists("Name")   random(lo, hi)
  get_global("score") / set_global("score", v)   hud("text shown on screen")   log("debug")
Rules: use floats (1.0 not 1) for math with positions; `let` for locals; no semicolon after fn blocks needed.
Example player:
fn start() { this.speed = 6.0; this.jump = 8.0; }
fn update(dt) {
    if key("A") || key("LEFT")  { this.x -= this.speed * dt; }
    if key("D") || key("RIGHT") { this.x += this.speed * dt; }
    if key("W") || key("UP")    { this.z -= this.speed * dt; }
    if key("S") || key("DOWN")  { this.z += this.speed * dt; }
    if key_pressed("SPACE") && this.grounded { this.vy = this.jump; }
    let n = destroy_touching(this.name, "Coin");
    if n > 0 { set_global("score", get_global("score") + n); }
    hud("Score: " + get_global("score"));
    if this.y < -10.0 { this.x = 0.0; this.y = 2.0; this.z = 0.0; this.vy = 0.0; }
}
"##;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub args: Value,
}

/// Extract tool calls from model text: ```tool blocks, ```json blocks with "tool", or bare JSON lines.
pub fn parse_tool_calls(text: &str) -> Vec<ToolCall> {
    let mut out = vec![];
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let Some(end) = after.find("```") else { break };
        let block = &after[..end];
        let (lang, body) = block.split_once('\n').unwrap_or(("", block));
        let lang = lang.trim().to_ascii_lowercase();
        if lang == "tool" || lang == "json" || lang.is_empty() || lang == "tool_call" {
            collect_calls(body.trim(), &mut out);
        }
        rest = &after[end + 3..];
    }
    if out.is_empty() {
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with('{') && l.contains("\"tool\"") {
                collect_calls(l, &mut out);
            }
        }
    }
    out
}

fn collect_calls(body: &str, out: &mut Vec<ToolCall>) {
    let push = |v: &Value, out: &mut Vec<ToolCall>| {
        let name = v
            .get("tool")
            .or_else(|| v.get("name"))
            .and_then(|n| n.as_str());
        if let Some(name) = name {
            let args = v
                .get("args")
                .or_else(|| v.get("arguments"))
                .cloned()
                .unwrap_or(json!({}));
            let args = match args {
                Value::String(s) => serde_json::from_str(&s).unwrap_or(json!({})),
                a => a,
            };
            out.push(ToolCall {
                name: name.to_string(),
                args,
            });
        }
    };
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Array(a)) => a.iter().for_each(|v| push(v, out)),
        Ok(v) => push(&v, out),
        Err(_) => {
            // Several JSON objects in one block, one per line.
            for l in body.lines() {
                if let Ok(v) = serde_json::from_str::<Value>(l.trim()) {
                    push(&v, out);
                }
            }
        }
    }
}

pub fn strip_tool_blocks(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("```tool") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 7..];
        match after.find("```") {
            Some(end) => rest = &after[end + 3..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

// ---------------------------------------------------------------- executor

fn vec3(v: &Value) -> Option<Vec3> {
    if let Some(a) = v.as_array() {
        let f: Vec<f32> = a
            .iter()
            .filter_map(|x| x.as_f64())
            .map(|x| x as f32)
            .collect();
        return (f.len() >= 3).then(|| Vec3::new(f[0], f[1], f[2]));
    }
    if let Some(o) = v.as_object() {
        let g = |k: &str| o.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
        return Some(Vec3::new(g("x"), g("y"), g("z")));
    }
    v.as_f64().map(|s| Vec3::splat(s as f32))
}

fn target(scene: &Scene, args: &Value) -> Result<EntityId, String> {
    let key = args
        .get("target")
        .or_else(|| args.get("name"))
        .or_else(|| args.get("id"))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .ok_or("missing 'target'")?;
    scene
        .find(&key)
        .ok_or_else(|| format!("no entity named '{key}'"))
}

/// Apply the shared create/update fields onto an entity.
fn apply_fields(scene: &mut Scene, id: EntityId, a: &Value) -> Result<(), String> {
    let parent = match a.get("parent").and_then(|p| p.as_str()) {
        Some("") | None => None,
        Some(p) => Some(
            scene
                .find(p)
                .ok_or_else(|| format!("parent '{p}' not found"))?,
        ),
    };
    let e = scene.get_mut(id).unwrap();
    if let Some(m) = a.get("mesh").and_then(|m| m.as_str()) {
        e.mesh = MeshKind::parse(m);
    }
    if let Some(p) = a.get("position").or_else(|| a.get("pos")).and_then(vec3) {
        e.transform.position = p;
    }
    if let Some(r) = a.get("rotation").or_else(|| a.get("rot")).and_then(vec3) {
        e.transform.rotation = r;
    }
    if let Some(s) = a.get("scale").and_then(vec3) {
        e.transform.scale = s;
    }
    if let Some(c) = a.get("color").and_then(parse_color) {
        e.color = c;
    }
    if let Some(t) = a.get("tag").and_then(|t| t.as_str()) {
        e.tag = t.to_string();
    }
    if let Some(b) = a.get("active").and_then(|b| b.as_bool()) {
        e.active = b;
    }
    if let Some(b) = a.get("collider").and_then(|b| b.as_bool()) {
        e.collider = b;
    }
    if let Some(b) = a.get("trigger").and_then(|b| b.as_bool()) {
        e.trigger = b;
    }
    if let Some(s) = a.get("script").and_then(|s| s.as_str()) {
        e.script = s.to_string();
    }
    if parent.is_some() {
        e.parent = parent;
    }
    match a.get("rigidbody") {
        Some(Value::Bool(false)) | Some(Value::Null) => e.rigidbody = None,
        Some(Value::Bool(true)) => e.rigidbody = Some(Rigidbody::default()),
        Some(Value::Object(o)) => {
            let mut rb = e.rigidbody.clone().unwrap_or_default();
            if let Some(m) = o.get("mass").and_then(|v| v.as_f64()) {
                rb.mass = m as f32;
            }
            if let Some(g) = o
                .get("gravity")
                .or_else(|| o.get("use_gravity"))
                .and_then(|v| v.as_bool())
            {
                rb.use_gravity = g;
            }
            if let Some(s) = o
                .get("static")
                .or_else(|| o.get("is_static"))
                .and_then(|v| v.as_bool())
            {
                rb.is_static = s;
            }
            if let Some(b) = o.get("bounciness").and_then(|v| v.as_f64()) {
                rb.bounciness = b as f32;
            }
            e.rigidbody = Some(rb);
        }
        _ => {}
    }
    match a.get("light") {
        Some(Value::Bool(true)) => e.light = Some(Light::default()),
        Some(Value::Bool(false)) | Some(Value::Null) => e.light = None,
        Some(Value::String(k)) if k.eq_ignore_ascii_case("point") => e.light = Some(Light::point()),
        Some(Value::String(_)) => e.light = Some(Light::default()),
        _ => {}
    }
    if let Some(l) = &mut e.light {
        if let Some(c) = a.get("light_color").and_then(parse_color) {
            l.color = c;
        }
        if let Some(v) = a.get("intensity").and_then(|v| v.as_f64()) {
            l.intensity = v as f32;
        }
        if let Some(v) = a.get("range").and_then(|v| v.as_f64()) {
            l.range = v as f32;
        }
    }
    if let Some(l) = a.get("layer").and_then(|l| l.as_str()) {
        e.layer = l.to_string();
    }
    if let Some(b) = a.get("static").and_then(|b| b.as_bool()) {
        e.is_static = b;
    }
    let m = a.get("material").cloned().unwrap_or(Value::Null);
    let mat = |k: &str| m.get(k).or_else(|| a.get(k));
    let mt = &mut e.material;
    if let Some(v) = mat("metallic").and_then(|v| v.as_f64()) {
        mt.metallic = v.clamp(0.0, 1.0) as f32;
    }
    if let Some(v) = mat("roughness").and_then(|v| v.as_f64()) {
        mt.roughness = v.clamp(0.0, 1.0) as f32;
    }
    if let Some(v) = mat("emission") {
        if let Some(c) = parse_color(v) {
            mt.emission = c;
            if mt.emission_strength == 0.0 {
                mt.emission_strength = 1.0;
            }
        }
    }
    if let Some(v) = mat("emission_strength").and_then(|v| v.as_f64()) {
        mt.emission_strength = v as f32;
    }
    if let Some(v) = mat("tiling").and_then(|v| v.as_f64()) {
        mt.tiling = v as f32;
    }
    if let Some(v) = mat("texture").and_then(|v| v.as_str()) {
        mt.texture = v.to_string();
    }
    if let Some(v) = mat("unlit").and_then(|v| v.as_bool()) {
        mt.unlit = v;
    }
    if let Some(v) = mat("pattern").and_then(|v| v.as_str()) {
        mt.pattern = match v.to_ascii_lowercase().as_str() {
            "checker" => Pattern::Checker,
            "grid" => Pattern::Grid,
            _ => Pattern::None,
        };
    }
    Ok(())
}

fn create(scene: &mut Scene, a: &Value) -> Result<EntityId, String> {
    let name = a.get("name").and_then(|n| n.as_str()).unwrap_or("Entity");
    let mesh = MeshKind::parse(a.get("mesh").and_then(|m| m.as_str()).unwrap_or("cube"));
    let id = scene.spawn(name, mesh);
    if let Err(e) = apply_fields(scene, id, a) {
        scene.remove(id);
        return Err(e);
    }
    Ok(id)
}

pub struct ToolOutcome {
    pub result: String,
    pub select: Option<EntityId>,
}

/// Tools that change or delete things and therefore ask the user first (opencode "ask" permission).
pub const DESTRUCTIVE: &[&str] = &["delete_entity", "clear_scene", "write_file", "delete_file"];

/// Tools that modify the scene or project (counted as "changes" in the agent panel).
pub fn is_mutation(name: &str) -> bool {
    !matches!(
        name,
        "get_scene"
            | "select"
            | "list_files"
            | "read_file"
            | "grep"
            | "web_search"
            | "fetch_url"
            | "memory_search"
            | "remember"
            | "task"
            | "todo_write"
    )
}

/// Resolve a project-relative path, refusing anything that escapes the project folder.
pub fn project_path(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim().trim_start_matches(['/', '\\']);
    let p = Path::new(rel);
    if p.is_absolute()
        || p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return Err("path must stay inside the project folder".into());
    }
    Ok(root.join(p))
}

/// Executes scene and project-file tools. Orchestrator tools (web, memory, task…) are not handled here.
pub fn execute_in(
    scene: &mut Scene,
    call: &ToolCall,
    root: Option<&Path>,
) -> Result<ToolOutcome, String> {
    let a = &call.args;
    let ok = |s: String| {
        Ok(ToolOutcome {
            result: s,
            select: None,
        })
    };
    let need_root =
        || root.ok_or_else(|| "save the scene first so it has a project folder".to_string());
    let arg = |k: &str| a.get(k).and_then(|v| v.as_str()).unwrap_or_default();
    match call.name.as_str() {
        "list_files" => {
            let r = need_root()?;
            let dir = project_path(r, arg("path"))?;
            let mut out = vec![];
            walk(&dir, r, 3, &mut out);
            ok(if out.is_empty() {
                "(empty)".into()
            } else {
                out.join("\n")
            })
        }
        "read_file" => {
            let p = project_path(need_root()?, arg("path"))?;
            let s = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", arg("path")))?;
            ok(s.chars().take(20_000).collect())
        }
        "write_file" => {
            let p = project_path(need_root()?, arg("path"))?;
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            std::fs::write(&p, arg("content")).map_err(|e| e.to_string())?;
            ok(format!(
                "wrote {} ({} bytes)",
                arg("path"),
                arg("content").len()
            ))
        }
        "delete_file" => {
            let p = project_path(need_root()?, arg("path"))?;
            std::fs::remove_file(&p).map_err(|e| e.to_string())?;
            ok(format!("deleted {}", arg("path")))
        }
        "grep" => {
            let r = need_root()?;
            let pat = arg("pattern").to_lowercase();
            let mut files = vec![];
            walk(r, r, 4, &mut files);
            let mut hits = vec![];
            for f in files.iter().filter(|f| {
                f.ends_with(".rhai")
                    || f.ends_with(".json")
                    || f.ends_with(".md")
                    || f.ends_with(".txt")
            }) {
                if let Ok(s) = std::fs::read_to_string(r.join(f)) {
                    for (i, l) in s.lines().enumerate() {
                        if l.to_lowercase().contains(&pat) {
                            hits.push(format!(
                                "{f}:{}: {}",
                                i + 1,
                                l.trim().chars().take(160).collect::<String>()
                            ));
                        }
                    }
                }
            }
            for e in &scene.entities {
                for (i, l) in e.script.lines().enumerate() {
                    if l.to_lowercase().contains(&pat) {
                        hits.push(format!("<scene:{}>:{}: {}", e.name, i + 1, l.trim()));
                    }
                }
            }
            hits.truncate(80);
            ok(if hits.is_empty() {
                "no matches".into()
            } else {
                hits.join("\n")
            })
        }
        "set_script" if a.get("file").is_some() && a.get("code").is_none() => {
            let p = project_path(need_root()?, arg("file"))?;
            let code = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
            let mut c = call.clone();
            c.args = json!({"target": a.get("target").cloned().unwrap_or_default(), "code": code});
            execute(scene, &c)
        }
        _ => execute(scene, call),
    }
}

fn walk(dir: &Path, root: &Path, depth: u32, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let rel = p
            .strip_prefix(root)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        if p.is_dir() {
            out.push(format!("{rel}/"));
            if depth > 0 {
                walk(&p, root, depth - 1, out);
            }
        } else {
            out.push(rel);
        }
        if out.len() > 400 {
            return;
        }
    }
}

/// Executes a scene tool. Orchestrator tools are not handled here.
pub fn execute(scene: &mut Scene, call: &ToolCall) -> Result<ToolOutcome, String> {
    let a = &call.args;
    let ok = |s: String| {
        Ok(ToolOutcome {
            result: s,
            select: None,
        })
    };
    match call.name.as_str() {
        "get_scene" => ok(scene.summary()),
        "create_entity" => {
            let id = create(scene, a)?;
            let n = scene.get(id).unwrap().name.clone();
            Ok(ToolOutcome {
                result: format!("created '{n}' (id {id})"),
                select: Some(id),
            })
        }
        "create_many" => {
            let list = a
                .get("entities")
                .and_then(|e| e.as_array())
                .ok_or("missing 'entities' array")?;
            let mut names = vec![];
            let mut errs = vec![];
            for item in list {
                match create(scene, item) {
                    Ok(id) => names.push(scene.get(id).unwrap().name.clone()),
                    Err(e) => errs.push(e),
                }
            }
            let mut r = format!("created {} entities: {}", names.len(), names.join(", "));
            if !errs.is_empty() {
                r.push_str(&format!(" | errors: {}", errs.join("; ")));
            }
            ok(r)
        }
        "update_entity" | "set_transform" | "add_component" => {
            let id = target(scene, a)?;
            if let Some(n) = a.get("new_name").and_then(|n| n.as_str()) {
                let n = scene.unique_name(n);
                scene.get_mut(id).unwrap().name = n;
            }
            apply_fields(scene, id, a)?;
            ok(format!("updated '{}'", scene.get(id).unwrap().name))
        }
        "delete_entity" => {
            let id = target(scene, a)?;
            let n = scene.get(id).unwrap().name.clone();
            scene.remove(id);
            ok(format!("deleted '{n}'"))
        }
        "duplicate_entity" => {
            let id = target(scene, a)?;
            let count = a
                .get("count")
                .and_then(|c| c.as_u64())
                .unwrap_or(1)
                .min(200);
            let offset = a.get("offset").and_then(vec3).unwrap_or(Vec3::X * 2.0);
            let base = scene.get(id).unwrap().transform.position;
            let mut names = vec![];
            for i in 1..=count {
                let nid = scene.duplicate(id).ok_or("duplicate failed")?;
                let e = scene.get_mut(nid).unwrap();
                e.transform.position = base + offset * i as f32;
                names.push(e.name.clone());
            }
            ok(format!("created {}", names.join(", ")))
        }
        "set_script" => {
            let id = target(scene, a)?;
            let code = a
                .get("code")
                .or_else(|| a.get("script"))
                .and_then(|c| c.as_str())
                .ok_or("missing 'code'")?;
            rhai::Engine::new()
                .compile(code)
                .map_err(|e| format!("script does not compile: {e}"))?;
            scene.get_mut(id).unwrap().script = code.to_string();
            ok(format!(
                "script attached to '{}'",
                scene.get(id).unwrap().name
            ))
        }
        "add_camera" => {
            let name = a
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("Main Camera");
            let id = match scene
                .find(name)
                .filter(|id| scene.get(*id).unwrap().camera.is_some())
            {
                Some(id) => id,
                None => {
                    // Only one active main camera.
                    for e in scene.entities.iter_mut().filter(|e| e.camera.is_some()) {
                        e.active = false;
                    }
                    scene.spawn(name, MeshKind::None)
                }
            };
            apply_fields(scene, id, a)?;
            let e = scene.get_mut(id).unwrap();
            let mut cam = e.camera.clone().unwrap_or_default();
            if let Some(f) = a.get("fov").and_then(|f| f.as_f64()) {
                cam.fov = f as f32;
            }
            if let Some(f) = a.get("follow").and_then(|f| f.as_str()) {
                cam.follow = f.to_string();
            }
            e.camera = Some(cam);
            e.active = true;
            ok(format!("camera '{}' ready", e.name))
        }
        "set_environment" => {
            let env = &mut scene.environment;
            if let Some(c) = a.get("sky_top").and_then(parse_color) {
                env.sky_top = c;
            }
            if let Some(c) = a.get("sky_bottom").and_then(parse_color) {
                env.sky_bottom = c;
            }
            if let Some(v) = a.get("ambient").and_then(|v| v.as_f64()) {
                env.ambient = v as f32;
            }
            if let Some(v) = a.get("gravity").and_then(|v| v.as_f64()) {
                env.gravity = v as f32;
            }
            ok("environment updated".into())
        }
        "clear_scene" => {
            let keep: Vec<String> = a
                .get("keep")
                .and_then(|k| k.as_array())
                .map(|k| {
                    k.iter()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            scene
                .entities
                .retain(|e| keep.iter().any(|k| k.eq_ignore_ascii_case(&e.name)));
            ok(format!("scene cleared ({} kept)", scene.entities.len()))
        }
        "select" => {
            let id = target(scene, a)?;
            Ok(ToolOutcome {
                result: "selected".into(),
                select: Some(id),
            })
        }
        other => Err(format!("unknown tool '{other}'")),
    }
}
