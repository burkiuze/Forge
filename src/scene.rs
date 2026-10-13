//! Scene graph: entities with components, serialized as JSON.

use glam::{EulerRot, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

pub type EntityId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeshKind {
    None,
    Cube,
    Sphere,
    Plane,
    Cylinder,
    Cone,
    Capsule,
}

impl MeshKind {
    pub const ALL: [MeshKind; 7] = [
        MeshKind::None,
        MeshKind::Cube,
        MeshKind::Sphere,
        MeshKind::Plane,
        MeshKind::Cylinder,
        MeshKind::Cone,
        MeshKind::Capsule,
    ];
    pub fn label(self) -> &'static str {
        match self {
            MeshKind::None => "Empty",
            MeshKind::Cube => "Cube",
            MeshKind::Sphere => "Sphere",
            MeshKind::Plane => "Plane",
            MeshKind::Cylinder => "Cylinder",
            MeshKind::Cone => "Cone",
            MeshKind::Capsule => "Capsule",
        }
    }
    pub fn parse(s: &str) -> MeshKind {
        match s.to_ascii_lowercase().as_str() {
            "cube" | "box" => MeshKind::Cube,
            "sphere" | "ball" => MeshKind::Sphere,
            "plane" | "ground" | "floor" | "quad" => MeshKind::Plane,
            "cylinder" => MeshKind::Cylinder,
            "cone" => MeshKind::Cone,
            "capsule" => MeshKind::Capsule,
            _ => MeshKind::None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    pub position: Vec3,
    /// Euler angles in degrees (XYZ).
    pub rotation: Vec3,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self { position: Vec3::ZERO, rotation: Vec3::ZERO, scale: Vec3::ONE }
    }
}

impl Transform {
    pub fn quat(&self) -> Quat {
        let r = self.rotation * (std::f32::consts::PI / 180.0);
        Quat::from_euler(EulerRot::XYZ, r.x, r.y, r.z)
    }
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.quat(), self.position)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Rigidbody {
    pub mass: f32,
    pub use_gravity: bool,
    pub is_static: bool,
    pub bounciness: f32,
    #[serde(skip)]
    pub velocity: Vec3,
    #[serde(skip)]
    pub grounded: bool,
}

impl Default for Rigidbody {
    fn default() -> Self {
        Self { mass: 1.0, use_gravity: true, is_static: false, bounciness: 0.0, velocity: Vec3::ZERO, grounded: false }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Light {
    pub color: [f32; 3],
    pub intensity: f32,
}

impl Default for Light {
    fn default() -> Self {
        Self { color: [1.0, 0.96, 0.9], intensity: 1.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraComp {
    pub fov: f32,
    pub follow: String,
}

impl Default for CameraComp {
    fn default() -> Self {
        Self { fov: 60.0, follow: String::new() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Entity {
    pub id: EntityId,
    pub name: String,
    pub active: bool,
    pub parent: Option<EntityId>,
    pub tag: String,
    pub transform: Transform,
    pub mesh: MeshKind,
    pub color: [f32; 3],
    pub collider: bool,
    pub trigger: bool,
    pub rigidbody: Option<Rigidbody>,
    pub light: Option<Light>,
    pub camera: Option<CameraComp>,
    /// Rhai script source. Defines `fn start()` and/or `fn update(dt)`.
    pub script: String,
}

impl Default for Entity {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Entity".into(),
            active: true,
            parent: None,
            tag: String::new(),
            transform: Transform::default(),
            mesh: MeshKind::None,
            color: [0.8, 0.8, 0.8],
            collider: true,
            trigger: false,
            rigidbody: None,
            light: None,
            camera: None,
            script: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Environment {
    pub sky_top: [f32; 3],
    pub sky_bottom: [f32; 3],
    pub ambient: f32,
    pub gravity: f32,
    pub sun_dir: Vec3,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sky_top: [0.20, 0.27, 0.38],
            sky_bottom: [0.55, 0.50, 0.46],
            ambient: 0.35,
            gravity: -18.0,
            sun_dir: Vec3::new(-0.4, -1.0, -0.3),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Scene {
    pub name: String,
    pub next_id: EntityId,
    pub entities: Vec<Entity>,
    pub environment: Environment,
}

impl Default for Scene {
    fn default() -> Self {
        Self { name: "Untitled".into(), next_id: 1, entities: vec![], environment: Environment::default() }
    }
}

impl Scene {
    /// Default scene: ground, sun, camera, and a cube.
    pub fn starter() -> Self {
        let mut s = Scene { name: "Main".into(), ..Default::default() };
        let g = s.spawn("Ground", MeshKind::Plane);
        let e = s.get_mut(g).unwrap();
        e.transform.scale = Vec3::new(20.0, 1.0, 20.0);
        e.color = [0.32, 0.34, 0.36];
        let l = s.spawn("Sun", MeshKind::None);
        let e = s.get_mut(l).unwrap();
        e.light = Some(Light::default());
        e.transform.position = Vec3::new(0.0, 10.0, 0.0);
        e.transform.rotation = Vec3::new(-50.0, 30.0, 0.0);
        let c = s.spawn("Main Camera", MeshKind::None);
        let e = s.get_mut(c).unwrap();
        e.camera = Some(CameraComp::default());
        e.transform.position = Vec3::new(0.0, 6.0, 12.0);
        e.transform.rotation = Vec3::new(-20.0, 0.0, 0.0);
        let b = s.spawn("Cube", MeshKind::Cube);
        let e = s.get_mut(b).unwrap();
        e.transform.position = Vec3::new(0.0, 0.5, 0.0);
        e.color = [0.94, 0.35, 0.12];
        s
    }

    pub fn spawn(&mut self, name: &str, mesh: MeshKind) -> EntityId {
        let id = self.next_id;
        self.next_id += 1;
        let name = self.unique_name(name);
        self.entities.push(Entity { id, name, mesh, ..Default::default() });
        id
    }

    pub fn unique_name(&self, base: &str) -> String {
        let base = if base.trim().is_empty() { "Entity" } else { base.trim() };
        if !self.entities.iter().any(|e| e.name == base) {
            return base.to_string();
        }
        (1..).map(|i| format!("{base} ({i})")).find(|n| !self.entities.iter().any(|e| &e.name == n)).unwrap()
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.entities.iter().find(|e| e.id == id)
    }
    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.iter_mut().find(|e| e.id == id)
    }

    /// Resolve by numeric id or (case-insensitive) name.
    pub fn find(&self, key: &str) -> Option<EntityId> {
        let key = key.trim();
        if let Ok(id) = key.trim_start_matches('#').parse::<u64>() {
            if self.get(id).is_some() {
                return Some(id);
            }
        }
        self.entities
            .iter()
            .find(|e| e.name == key)
            .or_else(|| self.entities.iter().find(|e| e.name.eq_ignore_ascii_case(key)))
            .map(|e| e.id)
    }

    /// Removes an entity and all its descendants.
    pub fn remove(&mut self, id: EntityId) {
        let mut doomed = vec![id];
        let mut i = 0;
        while i < doomed.len() {
            let p = doomed[i];
            doomed.extend(self.entities.iter().filter(|e| e.parent == Some(p)).map(|e| e.id));
            i += 1;
        }
        self.entities.retain(|e| !doomed.contains(&e.id));
    }

    pub fn duplicate(&mut self, id: EntityId) -> Option<EntityId> {
        let mut e = self.get(id)?.clone();
        e.id = self.next_id;
        self.next_id += 1;
        e.name = self.unique_name(&e.name);
        e.transform.position.x += 1.0;
        let nid = e.id;
        self.entities.push(e);
        Some(nid)
    }

    /// World matrix including parent chain.
    pub fn world_matrix(&self, id: EntityId) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        let mut cur = self.get(id);
        let mut guard = 0;
        while let Some(e) = cur {
            m = e.transform.matrix() * m;
            cur = e.parent.and_then(|p| self.get(p));
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        m
    }

    pub fn is_active(&self, id: EntityId) -> bool {
        let mut cur = self.get(id);
        while let Some(e) = cur {
            if !e.active {
                return false;
            }
            cur = e.parent.and_then(|p| self.get(p));
        }
        true
    }

    pub fn children(&self, id: Option<EntityId>) -> Vec<EntityId> {
        self.entities.iter().filter(|e| e.parent == id).map(|e| e.id).collect()
    }

    pub fn sun_dir(&self) -> Vec3 {
        self.entities
            .iter()
            .find(|e| e.light.is_some() && e.active)
            .map(|e| e.transform.quat() * Vec3::NEG_Z)
            .unwrap_or(self.environment.sun_dir)
            .normalize_or(Vec3::NEG_Y)
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap())
    }

    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| e.to_string())
    }

    /// Compact JSON description used as AI context.
    pub fn summary(&self) -> String {
        let ents: Vec<serde_json::Value> = self
            .entities
            .iter()
            .map(|e| {
                let mut v = serde_json::json!({
                    "id": e.id,
                    "name": e.name,
                    "mesh": e.mesh.label().to_lowercase(),
                    "pos": round3(e.transform.position),
                    "rot": round3(e.transform.rotation),
                    "scale": round3(e.transform.scale),
                    "color": color_hex(e.color),
                });
                let o = v.as_object_mut().unwrap();
                if !e.active { o.insert("active".into(), false.into()); }
                if let Some(p) = e.parent { o.insert("parent".into(), p.into()); }
                if !e.tag.is_empty() { o.insert("tag".into(), e.tag.clone().into()); }
                if let Some(rb) = &e.rigidbody {
                    o.insert("rigidbody".into(), serde_json::json!({"mass": rb.mass, "gravity": rb.use_gravity, "static": rb.is_static}));
                }
                if e.trigger { o.insert("trigger".into(), true.into()); }
                if !e.collider { o.insert("collider".into(), false.into()); }
                if e.light.is_some() { o.insert("light".into(), true.into()); }
                if let Some(c) = &e.camera { o.insert("camera".into(), serde_json::json!({"fov": c.fov, "follow": c.follow})); }
                if !e.script.is_empty() { o.insert("script_lines".into(), e.script.lines().count().into()); }
                v
            })
            .collect();
        serde_json::json!({"scene": self.name, "gravity": self.environment.gravity, "entities": ents}).to_string()
    }
}

fn round3(v: Vec3) -> [f32; 3] {
    let r = |x: f32| (x * 100.0).round() / 100.0;
    [r(v.x), r(v.y), r(v.z)]
}

pub fn color_hex(c: [f32; 3]) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (c[0].clamp(0.0, 1.0) * 255.0) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0) as u8
    )
}

/// Parses `#rrggbb`, `rrggbb`, a few color names, or `[r,g,b]` (0..1 or 0..255).
pub fn parse_color(v: &serde_json::Value) -> Option<[f32; 3]> {
    if let Some(arr) = v.as_array() {
        let f: Vec<f32> = arr.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
        if f.len() >= 3 {
            let scale = if f.iter().any(|x| *x > 1.0) { 255.0 } else { 1.0 };
            return Some([f[0] / scale, f[1] / scale, f[2] / scale]);
        }
        return None;
    }
    let s = v.as_str()?.trim().to_ascii_lowercase();
    let named = match s.as_str() {
        "red" => Some("#e53935"), "green" => Some("#43a047"), "blue" => Some("#1e88e5"),
        "yellow" => Some("#fdd835"), "orange" => Some("#f05a1e"), "purple" => Some("#8e24aa"),
        "white" => Some("#f5f5f5"), "black" => Some("#202020"), "gray" | "grey" => Some("#808080"),
        "brown" => Some("#795548"), "cyan" => Some("#00bcd4"), "pink" => Some("#ec407a"),
        "gold" => Some("#ffc107"),
        _ => None,
    };
    let h = named.unwrap_or(&s).trim_start_matches('#').to_string();
    if h.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|x| x as f32 / 255.0);
    Some([p(0)?, p(2)?, p(4)?])
}
