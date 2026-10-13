//! GPU viewport renderer (OpenGL via glow) + cameras, projection and picking.

use crate::mesh;
use crate::scene::{EntityId, LightKind, MeshKind, Pattern, Scene};
use eframe::glow::{self, HasContext};
use glam::{Mat4, Vec3, Vec4};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- cameras

#[derive(Clone, Copy)]
pub struct CameraView {
    pub view: Mat4,
    pub proj: Mat4,
    pub eye: Vec3,
}

impl CameraView {
    pub fn view_proj(&self) -> Mat4 {
        self.proj * self.view
    }

    /// World → screen point inside `rect` (None if behind the camera).
    pub fn project(&self, p: Vec3, rect: egui::Rect) -> Option<egui::Pos2> {
        let c = self.view_proj() * p.extend(1.0);
        if c.w <= 0.01 {
            return None;
        }
        let n = c.truncate() / c.w;
        Some(egui::pos2(
            rect.left() + (n.x * 0.5 + 0.5) * rect.width(),
            rect.top() + (1.0 - (n.y * 0.5 + 0.5)) * rect.height(),
        ))
    }

    /// Screen position → world ray (origin, dir).
    pub fn ray(&self, pos: egui::Pos2, rect: egui::Rect) -> (Vec3, Vec3) {
        let nx = (pos.x - rect.left()) / rect.width() * 2.0 - 1.0;
        let ny = 1.0 - (pos.y - rect.top()) / rect.height() * 2.0;
        let inv = self.view_proj().inverse();
        let a = inv * Vec4::new(nx, ny, -1.0, 1.0);
        let b = inv * Vec4::new(nx, ny, 1.0, 1.0);
        let a = a.truncate() / a.w;
        let b = b.truncate() / b.w;
        (a, (b - a).normalize())
    }
}

#[derive(Clone)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 0.5, 0.0),
            yaw: 0.6,
            pitch: 0.5,
            distance: 16.0,
            fov: 55.0,
        }
    }
}

impl OrbitCamera {
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + Vec3::new(cp * sy, sp, cp * cy) * self.distance
    }
    pub fn view(&self, aspect: f32) -> CameraView {
        let eye = self.eye();
        CameraView {
            view: Mat4::look_at_rh(eye, self.target, Vec3::Y),
            proj: Mat4::perspective_rh_gl(self.fov.to_radians(), aspect, 0.05, 2000.0),
            eye,
        }
    }
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-1.54, 1.54);
    }
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let eye = self.eye();
        let fwd = (self.target - eye).normalize();
        let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(fwd);
        let k = self.distance * 0.0016;
        self.target += (-right * dx + up * dy) * k;
    }
    pub fn zoom(&mut self, amount: f32) {
        self.distance = (self.distance * (1.0 - amount * 0.0015)).clamp(0.5, 500.0);
    }
    /// WASD-style fly while holding right mouse.
    pub fn fly(&mut self, local: Vec3) {
        let eye = self.eye();
        let fwd = (self.target - eye).normalize();
        let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
        self.target += (fwd * local.z + right * local.x + Vec3::Y * local.y)
            * (self.distance * 0.04).max(0.08);
    }
}

/// Camera from a scene camera entity (looks down its local -Z).
pub fn entity_camera(scene: &Scene, id: EntityId, aspect: f32) -> Option<CameraView> {
    let e = scene.get(id)?;
    let cam = e.camera.as_ref()?;
    let m = scene.world_matrix(id);
    let (_, rot, pos) = m.to_scale_rotation_translation();
    let fwd = rot * Vec3::NEG_Z;
    let up = rot * Vec3::Y;
    Some(CameraView {
        view: Mat4::look_to_rh(pos, fwd, up),
        proj: Mat4::perspective_rh_gl(
            cam.fov.clamp(10.0, 150.0).to_radians(),
            aspect,
            0.05,
            2000.0,
        ),
        eye: pos,
    })
}

pub fn main_camera(scene: &Scene) -> Option<EntityId> {
    scene
        .entities
        .iter()
        .find(|e| e.camera.is_some() && e.active)
        .map(|e| e.id)
}

// ---------------------------------------------------------------- picking

/// Ray vs the entity's oriented unit box. Returns hit distance.
pub fn pick(scene: &Scene, origin: Vec3, dir: Vec3) -> Option<EntityId> {
    let mut best: Option<(f32, EntityId)> = None;
    for e in &scene.entities {
        if !scene.is_active(e.id) {
            continue;
        }
        let pad = if e.mesh == MeshKind::None { 0.35 } else { 0.0 };
        let m = scene.world_matrix(e.id);
        let inv = m.inverse();
        let o = inv.transform_point3(origin);
        let d = inv.transform_vector3(dir);
        let (lo, hi) = if e.mesh == MeshKind::Plane {
            (Vec3::new(-0.5, -0.01, -0.5), Vec3::new(0.5, 0.01, 0.5))
        } else {
            (Vec3::splat(-0.5 - pad), Vec3::splat(0.5 + pad))
        };
        if let Some(t) = ray_aabb(o, d, lo, hi) {
            let world_t = (m.transform_point3(o + d * t) - origin).length();
            // Prefer small objects over big floors when overlapping.
            let bias = if e.mesh == MeshKind::Plane { 0.5 } else { 0.0 };
            if best.map_or(true, |(bt, _)| world_t + bias < bt) {
                best = Some((world_t + bias, e.id));
            }
        }
    }
    best.map(|b| b.1)
}

pub fn ray_aabb(o: Vec3, d: Vec3, lo: Vec3, hi: Vec3) -> Option<f32> {
    let inv = Vec3::ONE / d;
    let t1 = (lo - o) * inv;
    let t2 = (hi - o) * inv;
    let tmin = t1.min(t2).max_element();
    let tmax = t1.max(t2).min_element();
    (tmax >= tmin.max(0.0)).then_some(tmin.max(0.0))
}

// ---------------------------------------------------------------- draw list

pub struct DrawItem {
    pub mesh: MeshKind,
    pub model: Mat4,
    pub color: [f32; 3],
    pub highlight: f32,
    pub metallic: f32,
    pub roughness: f32,
    pub emission: [f32; 3],
    pub pattern: Pattern,
    pub tiling: f32,
    pub unlit: bool,
    pub texture: Option<PathBuf>,
    /// glTF/GLB file for `MeshKind::Model`.
    pub asset: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Shading {
    #[default]
    Lit,
    Unlit,
    Wireframe,
}

pub struct DrawList {
    pub cam: CameraView,
    pub sun_dir: Vec3,
    pub sun_color: [f32; 3],
    pub ambient: f32,
    pub sky_top: [f32; 3],
    pub sky_bottom: [f32; 3],
    pub fog: [f32; 2],
    pub exposure: f32,
    pub shadows: bool,
    pub softness: f32,
    pub points: Vec<(Vec3, [f32; 3], f32)>,
    pub items: Vec<DrawItem>,
    /// Depth-tested lines: (a, b, color).
    pub lines: Vec<(Vec3, Vec3, [f32; 4])>,
    pub shading: Shading,
}

#[derive(Default, Clone, Copy)]
pub struct FrameStats {
    pub draw_calls: usize,
    pub triangles: usize,
    pub lights: usize,
}

impl DrawList {
    pub fn from_scene(
        scene: &Scene,
        cam: CameraView,
        selected: Option<EntityId>,
        grid: bool,
        project_root: Option<&Path>,
        shading: Shading,
    ) -> Self {
        let env = &scene.environment;
        let mut items = Vec::with_capacity(scene.entities.len());
        let mut points = vec![];
        for e in &scene.entities {
            if !scene.is_active(e.id) {
                continue;
            }
            if let Some(l) = &e.light {
                if l.kind == LightKind::Point && points.len() < 8 {
                    let p = scene.world_matrix(e.id).w_axis.truncate();
                    points.push((
                        p,
                        [
                            l.color[0] * l.intensity,
                            l.color[1] * l.intensity,
                            l.color[2] * l.intensity,
                        ],
                        l.range.max(0.1),
                    ));
                }
            }
            if e.mesh == MeshKind::None {
                continue;
            }
            let m = &e.material;
            let texture = (!m.texture.is_empty()).then(|| match project_root {
                Some(r) => r.join(&m.texture),
                None => PathBuf::from(&m.texture),
            });
            items.push(DrawItem {
                mesh: e.mesh,
                model: scene.world_matrix(e.id),
                color: e.color,
                highlight: if Some(e.id) == selected { 1.0 } else { 0.0 },
                metallic: m.metallic,
                roughness: m.roughness,
                emission: [
                    m.emission[0] * m.emission_strength,
                    m.emission[1] * m.emission_strength,
                    m.emission[2] * m.emission_strength,
                ],
                pattern: if e.mesh == MeshKind::Plane
                    && m.pattern == Pattern::None
                    && m.texture.is_empty()
                {
                    Pattern::Checker
                } else {
                    m.pattern
                },
                tiling: m.tiling.max(0.01),
                unlit: m.unlit,
                texture,
                asset: (e.mesh == MeshKind::Model && !e.model.is_empty()).then(
                    || match project_root {
                        Some(r) => r.join(&e.model),
                        None => PathBuf::from(&e.model),
                    },
                ),
            });
        }
        let mut lines = vec![];
        if grid {
            let n = 40;
            for i in -n..=n {
                let a = if i == 0 {
                    0.45
                } else if i % 10 == 0 {
                    0.18
                } else {
                    0.07
                };
                let f = i as f32;
                let nn = n as f32;
                let cx = if i == 0 {
                    [0.35, 0.5, 1.0, a]
                } else {
                    [1.0, 1.0, 1.0, a]
                };
                let cz = if i == 0 {
                    [1.0, 0.35, 0.3, a]
                } else {
                    [1.0, 1.0, 1.0, a]
                };
                lines.push((Vec3::new(f, 0.003, -nn), Vec3::new(f, 0.003, nn), cx));
                lines.push((Vec3::new(-nn, 0.003, f), Vec3::new(nn, 0.003, f), cz));
            }
        }
        if let Some(sel) = selected.and_then(|s| scene.get(s)) {
            let m = scene.world_matrix(sel.id);
            let c = [1.0, 0.55, 0.15, 1.0];
            if sel.mesh != MeshKind::None {
                let corners: Vec<Vec3> = (0..8)
                    .map(|i| {
                        let p = Vec3::new(
                            if i & 1 == 0 { -0.5 } else { 0.5 },
                            if i & 2 == 0 { -0.5 } else { 0.5 },
                            if i & 4 == 0 { -0.5 } else { 0.5 },
                        );
                        let p = if sel.mesh == MeshKind::Plane {
                            p * Vec3::new(1.0, 0.0, 1.0)
                        } else {
                            p
                        };
                        m.transform_point3(p * 1.01)
                    })
                    .collect();
                for (a, b) in [
                    (0, 1),
                    (2, 3),
                    (4, 5),
                    (6, 7),
                    (0, 2),
                    (1, 3),
                    (4, 6),
                    (5, 7),
                    (0, 4),
                    (1, 5),
                    (2, 6),
                    (3, 7),
                ] {
                    lines.push((corners[a], corners[b], c));
                }
            }
            // Point light range ring.
            if let Some(l) = sel.light.as_ref().filter(|l| l.kind == LightKind::Point) {
                let p = m.w_axis.truncate();
                for k in 0..48 {
                    let a0 = k as f32 / 48.0 * std::f32::consts::TAU;
                    let a1 = (k + 1) as f32 / 48.0 * std::f32::consts::TAU;
                    let ring = |a: f32| p + Vec3::new(a.cos(), 0.0, a.sin()) * l.range;
                    lines.push((ring(a0), ring(a1), [1.0, 0.85, 0.4, 0.8]));
                }
            }
            // Camera frustum preview.
            if sel.camera.is_some() {
                if let Some(cv) = entity_camera(scene, sel.id, 16.0 / 9.0) {
                    let inv = cv.view_proj().inverse();
                    let corner = |x: f32, y: f32, z: f32| {
                        let v = inv * Vec4::new(x, y, z, 1.0);
                        v.truncate() / v.w
                    };
                    let near: Vec<Vec3> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                        .iter()
                        .map(|(x, y)| corner(*x, *y, -1.0))
                        .collect();
                    let far: Vec<Vec3> = near
                        .iter()
                        .map(|n| cv.eye + (*n - cv.eye).normalize() * 4.0)
                        .collect();
                    let cc = [0.85, 0.85, 0.95, 0.9];
                    for i in 0..4 {
                        lines.push((cv.eye, far[i], cc));
                        lines.push((far[i], far[(i + 1) % 4], cc));
                    }
                }
            }
        }
        Self {
            cam,
            sun_dir: scene.sun_dir(),
            sun_color: scene.sun_color(),
            ambient: env.ambient,
            sky_top: env.sky_top,
            sky_bottom: env.sky_bottom,
            fog: [env.fog_start, env.fog_end.max(env.fog_start + 1.0)],
            exposure: env.exposure,
            shadows: env.shadows,
            softness: env.shadow_softness,
            points,
            items,
            lines,
            shading,
        }
    }

    pub fn stats(&self) -> FrameStats {
        let tris: usize = self.items.iter().map(|i| mesh::triangles(i.mesh)).sum();
        FrameStats {
            draw_calls: self.items.len() * if self.shadows { 2 } else { 1 } + 2,
            triangles: tris * if self.shadows { 2 } else { 1 },
            lights: self.points.len() + 1,
        }
    }

    /// Light-space matrix that fits the shadow casters.
    fn light_matrix(&self) -> Mat4 {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for it in &self.items {
            let p = it.model.w_axis.truncate();
            let s = it
                .model
                .x_axis
                .truncate()
                .length()
                .max(it.model.y_axis.truncate().length())
                .max(it.model.z_axis.truncate().length());
            lo = lo.min(p - Vec3::splat(s * 0.75));
            hi = hi.max(p + Vec3::splat(s * 0.75));
        }
        if lo.x > hi.x {
            return Mat4::IDENTITY;
        }
        let center = (lo + hi) * 0.5;
        let radius = ((hi - lo).length() * 0.5).clamp(2.0, 120.0);
        let dir = self.sun_dir.normalize_or(Vec3::NEG_Y);
        let up = if dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        let view = Mat4::look_at_rh(center - dir * radius * 2.0, center, up);
        let proj = Mat4::orthographic_rh_gl(-radius, radius, -radius, radius, 0.1, radius * 4.0);
        proj * view
    }
}

// ---------------------------------------------------------------- GL renderer

const MESH_VS: &str = r#"
layout(location=0) in vec3 a_pos;
layout(location=1) in vec3 a_nrm;
layout(location=2) in vec2 a_uv;
uniform mat4 u_vp;
uniform mat4 u_model;
uniform mat4 u_light_vp;
out vec3 v_world;
out vec3 v_nrm;
out vec2 v_uv;
out vec4 v_light;
void main() {
    vec4 w = u_model * vec4(a_pos, 1.0);
    v_world = w.xyz;
    v_uv = a_uv;
    v_nrm = mat3(transpose(inverse(u_model))) * a_nrm;
    v_light = u_light_vp * w;
    gl_Position = u_vp * w;
}
"#;

const MESH_FS: &str = r#"
in vec3 v_world;
in vec3 v_nrm;
in vec2 v_uv;
in vec4 v_light;
uniform vec3 u_color;
uniform vec3 u_sun;
uniform vec3 u_sun_color;
uniform vec3 u_eye;
uniform vec3 u_sky_top;
uniform vec3 u_sky_bottom;
uniform float u_ambient;
uniform float u_highlight;
uniform int u_pattern;
uniform float u_tiling;
uniform float u_metallic;
uniform float u_roughness;
uniform vec3 u_emission;
uniform int u_unlit;
uniform int u_has_tex;
uniform sampler2D u_tex;
uniform int u_shadows;
uniform float u_soft;
uniform sampler2D u_shadow;
uniform vec2 u_fog;
uniform float u_exposure;
uniform int u_npoints;
uniform vec3 u_ppos[8];
uniform vec3 u_pcol[8];
uniform float u_prange[8];
out vec4 o_color;

float shadow_factor(vec3 n, vec3 l) {
    if (u_shadows == 0) return 1.0;
    vec3 p = v_light.xyz / v_light.w * 0.5 + 0.5;
    if (p.x < 0.0 || p.x > 1.0 || p.y < 0.0 || p.y > 1.0 || p.z > 1.0) return 1.0;
    float bias = max(0.0025 * (1.0 - dot(n, l)), 0.0006);
    vec2 texel = u_soft / vec2(textureSize(u_shadow, 0));
    float lit = 0.0;
    for (int x = -1; x <= 1; x++)
        for (int y = -1; y <= 1; y++)
            lit += (p.z - bias) > texture(u_shadow, p.xy + vec2(x, y) * texel).r ? 0.0 : 1.0;
    return lit / 9.0;
}

void main() {
    vec3 n = normalize(v_nrm);
    if (!gl_FrontFacing) n = -n;
    vec3 base = u_color;
    vec2 uv = v_uv * u_tiling;
    if (u_has_tex == 1) base *= texture(u_tex, uv).rgb;
    if (u_pattern == 1) {
        vec2 c = floor(v_world.xz * u_tiling);
        base *= mix(0.86, 1.06, mod(c.x + c.y, 2.0));
    } else if (u_pattern == 2) {
        vec2 g = abs(fract(v_world.xz * u_tiling) - 0.5);
        base *= (min(g.x, g.y) > 0.47) ? 1.0 : 0.72;
    }
    if (u_unlit == 1) {
        o_color = vec4(base + u_emission, 1.0);
        return;
    }
    vec3 v = normalize(u_eye - v_world);
    vec3 l = normalize(-u_sun);
    float ndl = max(dot(n, l), 0.0);
    float shininess = mix(256.0, 6.0, clamp(u_roughness, 0.0, 1.0));
    vec3 spec_col = mix(vec3(0.04), base, u_metallic);
    vec3 diffuse_col = base * (1.0 - u_metallic * 0.9);
    vec3 h = normalize(l + v);
    float spec_norm = (shininess + 8.0) / 25.0;
    float sh = shadow_factor(n, l);
    vec3 col = (diffuse_col * ndl + spec_col * pow(max(dot(n, h), 0.0), shininess) * spec_norm * ndl) * u_sun_color * sh;
    for (int i = 0; i < 8; i++) {
        if (i >= u_npoints) break;
        vec3 d = u_ppos[i] - v_world;
        float dist = length(d);
        vec3 pl = d / max(dist, 1e-4);
        float att = clamp(1.0 - dist / u_prange[i], 0.0, 1.0);
        att *= att;
        float pn = max(dot(n, pl), 0.0);
        vec3 ph = normalize(pl + v);
        col += (diffuse_col * pn + spec_col * pow(max(dot(n, ph), 0.0), shininess) * spec_norm * pn) * u_pcol[i] * att;
    }
    vec3 hemi = mix(u_sky_bottom, u_sky_top, n.y * 0.5 + 0.5);
    col += diffuse_col * hemi * u_ambient * 1.5;
    // Cheap environment reflection for metals.
    vec3 r = reflect(-v, n);
    col += spec_col * mix(u_sky_bottom, u_sky_top, r.y * 0.5 + 0.5) * u_metallic * (1.0 - u_roughness) * 0.8;
    col += u_emission;
    float rim = pow(1.0 - max(dot(n, v), 0.0), 3.0);
    col += vec3(1.0, 0.45, 0.1) * rim * u_highlight * 0.9;
    float dist = length(u_eye - v_world);
    col = mix(col, u_sky_bottom, clamp((dist - u_fog.x) / (u_fog.y - u_fog.x), 0.0, 1.0));
    col *= u_exposure;
    col = col / (col + vec3(0.85)) * 1.6;
    o_color = vec4(pow(col, vec3(1.0 / 1.15)), 1.0);
}
"#;

const DEPTH_VS: &str = r#"
layout(location=0) in vec3 a_pos;
uniform mat4 u_light_vp;
uniform mat4 u_model;
void main() { gl_Position = u_light_vp * u_model * vec4(a_pos, 1.0); }
"#;

const DEPTH_FS: &str = r#"
out vec4 o_color;
void main() { o_color = vec4(1.0); }
"#;

const LINE_VS: &str = r#"
layout(location=0) in vec3 a_pos;
layout(location=1) in vec4 a_col;
uniform mat4 u_vp;
out vec4 v_col;
void main() { v_col = a_col; gl_Position = u_vp * vec4(a_pos, 1.0); }
"#;

const LINE_FS: &str = r#"
in vec4 v_col;
out vec4 o_color;
void main() { o_color = v_col; }
"#;

const SKY_VS: &str = r#"
out vec2 v_p;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_p = p * 2.0 - 1.0;
    gl_Position = vec4(v_p, 0.999, 1.0);
}
"#;

const SKY_FS: &str = r#"
in vec2 v_p;
uniform mat4 u_inv_vp;
uniform vec3 u_sky_top;
uniform vec3 u_sky_bottom;
uniform vec3 u_sun;
uniform vec3 u_sun_color;
out vec4 o_color;
void main() {
    vec4 a = u_inv_vp * vec4(v_p, -1.0, 1.0);
    vec4 b = u_inv_vp * vec4(v_p, 1.0, 1.0);
    vec3 d = normalize(b.xyz / b.w - a.xyz / a.w);
    float t = clamp(d.y * 1.6 + 0.15, 0.0, 1.0);
    vec3 col = mix(u_sky_bottom, u_sky_top, t);
    float s = max(dot(d, normalize(-u_sun)), 0.0);
    col += u_sun_color * (pow(s, 600.0) * 4.0 + pow(s, 12.0) * 0.12);
    if (d.y < 0.0) col = mix(col, u_sky_bottom * 0.85, clamp(-d.y * 4.0, 0.0, 1.0));
    o_color = vec4(col, 1.0);
}
"#;

const SHADOW_SIZE: i32 = 2048;

struct GpuMesh {
    vao: glow::VertexArray,
    _vbo: glow::Buffer,
    count: i32,
}

pub struct GlRenderer {
    mesh_prog: glow::Program,
    depth_prog: glow::Program,
    line_prog: glow::Program,
    sky_prog: glow::Program,
    sky_vao: glow::VertexArray,
    meshes: HashMap<MeshKind, GpuMesh>,
    line_vao: glow::VertexArray,
    line_vbo: glow::Buffer,
    shadow_fbo: Option<glow::Framebuffer>,
    shadow_tex: Option<glow::Texture>,
    textures: HashMap<PathBuf, Option<glow::Texture>>,
    models: HashMap<PathBuf, Option<(GpuMesh, Option<glow::Texture>)>>,
}

fn as_bytes(v: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding/invalid bit patterns; lifetime tied to `v`.
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

unsafe fn compile(gl: &glow::Context, vs: &str, fs: &str) -> Result<glow::Program, String> {
    let header = "#version 330 core\nprecision highp float;\n";
    let prog = gl.create_program()?;
    let mut shaders = vec![];
    for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
        let sh = gl.create_shader(kind)?;
        gl.shader_source(sh, &format!("{header}{src}"));
        gl.compile_shader(sh);
        if !gl.get_shader_compile_status(sh) {
            return Err(gl.get_shader_info_log(sh));
        }
        gl.attach_shader(prog, sh);
        shaders.push(sh);
    }
    gl.link_program(prog);
    if !gl.get_program_link_status(prog) {
        return Err(gl.get_program_info_log(prog));
    }
    for sh in shaders {
        gl.detach_shader(prog, sh);
        gl.delete_shader(sh);
    }
    Ok(prog)
}

impl GlRenderer {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let mesh_prog = compile(gl, MESH_VS, MESH_FS)?;
            let depth_prog = compile(gl, DEPTH_VS, DEPTH_FS)?;
            let line_prog = compile(gl, LINE_VS, LINE_FS)?;
            let sky_prog = compile(gl, SKY_VS, SKY_FS)?;
            let sky_vao = gl.create_vertex_array()?;
            let mut meshes = HashMap::new();
            let stride = (mesh::STRIDE * 4) as i32;
            for kind in MeshKind::ALL {
                if kind == MeshKind::None {
                    continue;
                }
                let data = mesh::build(kind);
                let vao = gl.create_vertex_array()?;
                let vbo = gl.create_buffer()?;
                gl.bind_vertex_array(Some(vao));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                gl.buffer_data_u8_slice(
                    glow::ARRAY_BUFFER,
                    as_bytes(&data.verts),
                    glow::STATIC_DRAW,
                );
                gl.enable_vertex_attrib_array(0);
                gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);
                gl.enable_vertex_attrib_array(1);
                gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, stride, 12);
                gl.enable_vertex_attrib_array(2);
                gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, stride, 24);
                meshes.insert(
                    kind,
                    GpuMesh {
                        vao,
                        _vbo: vbo,
                        count: data.vertex_count(),
                    },
                );
            }
            let line_vao = gl.create_vertex_array()?;
            let line_vbo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(line_vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(line_vbo));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 28, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, 28, 12);
            gl.bind_vertex_array(None);

            // Shadow map (depth texture + FBO). Optional: shadows are skipped if unsupported.
            let (shadow_fbo, shadow_tex) = match (gl.create_framebuffer(), gl.create_texture()) {
                (Ok(fbo), Ok(tex)) => {
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                    gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        glow::DEPTH_COMPONENT24 as i32,
                        SHADOW_SIZE,
                        SHADOW_SIZE,
                        0,
                        glow::DEPTH_COMPONENT,
                        glow::UNSIGNED_INT,
                        glow::PixelUnpackData::Slice(None),
                    );
                    for (k, v) in [
                        (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                        (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                        (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                        (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                    ] {
                        gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
                    }
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                    gl.framebuffer_texture_2d(
                        glow::FRAMEBUFFER,
                        glow::DEPTH_ATTACHMENT,
                        glow::TEXTURE_2D,
                        Some(tex),
                        0,
                    );
                    gl.draw_buffer(glow::NONE);
                    gl.read_buffer(glow::NONE);
                    let ok = gl.check_framebuffer_status(glow::FRAMEBUFFER)
                        == glow::FRAMEBUFFER_COMPLETE;
                    gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                    gl.bind_texture(glow::TEXTURE_2D, None);
                    if ok {
                        (Some(fbo), Some(tex))
                    } else {
                        (None, None)
                    }
                }
                _ => (None, None),
            };
            Ok(Self {
                mesh_prog,
                depth_prog,
                line_prog,
                sky_prog,
                sky_vao,
                meshes,
                line_vao,
                line_vbo,
                shadow_fbo,
                shadow_tex,
                textures: HashMap::new(),
                models: HashMap::new(),
            })
        }
    }

    /// Load (once) an image file as a mip-mapped, repeating GL texture.
    unsafe fn texture(&mut self, gl: &glow::Context, path: &Path) -> Option<glow::Texture> {
        if let Some(t) = self.textures.get(path) {
            return *t;
        }
        let tex = image::open(path)
            .ok()
            .map(|img| img.to_rgba8())
            .and_then(|img| {
                let tex = gl.create_texture().ok()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    img.width() as i32,
                    img.height() as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(img.as_raw())),
                );
                gl.generate_mipmap(glow::TEXTURE_2D);
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR_MIPMAP_LINEAR as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::REPEAT as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::REPEAT as i32);
                Some(tex)
            });
        self.textures.insert(path.to_path_buf(), tex);
        tex
    }

    /// Forget a cached texture (e.g. after re-import).
    pub fn invalidate_textures(&mut self) {
        self.textures.clear();
        self.models.clear();
    }

    /// Vertex array, vertex count and built-in texture (models) for a draw item.
    unsafe fn geometry(
        &mut self,
        gl: &glow::Context,
        it: &DrawItem,
    ) -> Option<(glow::VertexArray, i32, Option<glow::Texture>)> {
        if it.mesh != MeshKind::Model {
            return self.meshes.get(&it.mesh).map(|m| (m.vao, m.count, None));
        }
        let path = it.asset.as_ref()?;
        if !self.models.contains_key(path) {
            let loaded = crate::model::load(path).ok().and_then(|data| {
                let vao = gl.create_vertex_array().ok()?;
                let vbo = gl.create_buffer().ok()?;
                let stride = (mesh::STRIDE * 4) as i32;
                gl.bind_vertex_array(Some(vao));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                gl.buffer_data_u8_slice(
                    glow::ARRAY_BUFFER,
                    as_bytes(&data.verts),
                    glow::STATIC_DRAW,
                );
                gl.enable_vertex_attrib_array(0);
                gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);
                gl.enable_vertex_attrib_array(1);
                gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, stride, 12);
                gl.enable_vertex_attrib_array(2);
                gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, stride, 24);
                gl.bind_vertex_array(None);
                let tex = data.texture.as_ref().and_then(|(w, h, px)| {
                    let t = gl.create_texture().ok()?;
                    gl.bind_texture(glow::TEXTURE_2D, Some(t));
                    gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        glow::RGBA8 as i32,
                        *w as i32,
                        *h as i32,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(Some(px)),
                    );
                    gl.generate_mipmap(glow::TEXTURE_2D);
                    gl.tex_parameter_i32(
                        glow::TEXTURE_2D,
                        glow::TEXTURE_MIN_FILTER,
                        glow::LINEAR_MIPMAP_LINEAR as i32,
                    );
                    gl.tex_parameter_i32(
                        glow::TEXTURE_2D,
                        glow::TEXTURE_MAG_FILTER,
                        glow::LINEAR as i32,
                    );
                    gl.tex_parameter_i32(
                        glow::TEXTURE_2D,
                        glow::TEXTURE_WRAP_S,
                        glow::REPEAT as i32,
                    );
                    gl.tex_parameter_i32(
                        glow::TEXTURE_2D,
                        glow::TEXTURE_WRAP_T,
                        glow::REPEAT as i32,
                    );
                    Some(t)
                });
                Some((
                    GpuMesh {
                        vao,
                        _vbo: vbo,
                        count: (data.verts.len() / mesh::STRIDE) as i32,
                    },
                    tex,
                ))
            });
            self.models.insert(path.clone(), loaded);
        }
        self.models
            .get(path)?
            .as_ref()
            .map(|(m, t)| (m.vao, m.count, *t))
    }

    pub fn paint(&mut self, gl: &glow::Context, dl: &DrawList, info: &egui::PaintCallbackInfo) {
        let vp = info.viewport_in_pixels();
        let clip = info.clip_rect_in_pixels();
        let light_vp = dl.light_matrix();
        let shadows = dl.shadows && self.shadow_fbo.is_some() && dl.shading == Shading::Lit;
        unsafe {
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::BLEND);
            gl.depth_mask(true);

            // 1. Shadow pass.
            if shadows {
                gl.disable(glow::SCISSOR_TEST);
                gl.bind_framebuffer(glow::FRAMEBUFFER, self.shadow_fbo);
                gl.viewport(0, 0, SHADOW_SIZE, SHADOW_SIZE);
                gl.clear_depth_f32(1.0);
                gl.clear(glow::DEPTH_BUFFER_BIT);
                gl.enable(glow::DEPTH_TEST);
                gl.depth_func(glow::LEQUAL);
                gl.enable(glow::POLYGON_OFFSET_FILL);
                gl.polygon_offset(1.5, 2.0);
                let p = self.depth_prog;
                gl.use_program(Some(p));
                gl.uniform_matrix_4_f32_slice(
                    gl.get_uniform_location(p, "u_light_vp").as_ref(),
                    false,
                    &light_vp.to_cols_array(),
                );
                let u_model = gl.get_uniform_location(p, "u_model");
                for it in dl.items.iter().filter(|i| !i.unlit) {
                    let Some((vao, count, _)) = self.geometry(gl, it) else {
                        continue;
                    };
                    gl.use_program(Some(p));
                    gl.uniform_matrix_4_f32_slice(
                        u_model.as_ref(),
                        false,
                        &it.model.to_cols_array(),
                    );
                    gl.bind_vertex_array(Some(vao));
                    gl.draw_arrays(glow::TRIANGLES, 0, count);
                }
                gl.disable(glow::POLYGON_OFFSET_FILL);
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            }

            gl.viewport(vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px);
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(
                clip.left_px,
                clip.from_bottom_px,
                clip.width_px,
                clip.height_px,
            );
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            let vpm = dl.cam.view_proj();

            // 2. Sky.
            gl.disable(glow::DEPTH_TEST);
            gl.use_program(Some(self.sky_prog));
            gl.uniform_matrix_4_f32_slice(
                gl.get_uniform_location(self.sky_prog, "u_inv_vp").as_ref(),
                false,
                &vpm.inverse().to_cols_array(),
            );
            u3(gl, self.sky_prog, "u_sky_top", dl.sky_top);
            u3(gl, self.sky_prog, "u_sky_bottom", dl.sky_bottom);
            u3(gl, self.sky_prog, "u_sun", dl.sun_dir.to_array());
            u3(gl, self.sky_prog, "u_sun_color", dl.sun_color);
            gl.bind_vertex_array(Some(self.sky_vao));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            // 3. Meshes.
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            if dl.shading == Shading::Wireframe {
                gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE);
            }
            let p = self.mesh_prog;
            gl.use_program(Some(p));
            gl.uniform_matrix_4_f32_slice(
                gl.get_uniform_location(p, "u_vp").as_ref(),
                false,
                &vpm.to_cols_array(),
            );
            gl.uniform_matrix_4_f32_slice(
                gl.get_uniform_location(p, "u_light_vp").as_ref(),
                false,
                &light_vp.to_cols_array(),
            );
            u3(gl, p, "u_sun", dl.sun_dir.to_array());
            u3(gl, p, "u_sun_color", dl.sun_color);
            u3(gl, p, "u_eye", dl.cam.eye.to_array());
            u3(gl, p, "u_sky_top", dl.sky_top);
            u3(gl, p, "u_sky_bottom", dl.sky_bottom);
            gl.uniform_1_f32(gl.get_uniform_location(p, "u_ambient").as_ref(), dl.ambient);
            gl.uniform_2_f32(
                gl.get_uniform_location(p, "u_fog").as_ref(),
                dl.fog[0],
                dl.fog[1],
            );
            gl.uniform_1_f32(
                gl.get_uniform_location(p, "u_exposure").as_ref(),
                dl.exposure,
            );
            gl.uniform_1_i32(
                gl.get_uniform_location(p, "u_shadows").as_ref(),
                shadows as i32,
            );
            gl.uniform_1_f32(
                gl.get_uniform_location(p, "u_soft").as_ref(),
                dl.softness.clamp(0.0, 4.0),
            );
            gl.uniform_1_i32(gl.get_uniform_location(p, "u_tex").as_ref(), 0);
            gl.uniform_1_i32(gl.get_uniform_location(p, "u_shadow").as_ref(), 1);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(
                glow::TEXTURE_2D,
                if shadows { self.shadow_tex } else { None },
            );
            gl.active_texture(glow::TEXTURE0);
            gl.uniform_1_i32(
                gl.get_uniform_location(p, "u_npoints").as_ref(),
                dl.points.len() as i32,
            );
            for (i, (pos, col, range)) in dl.points.iter().enumerate() {
                u3(gl, p, &format!("u_ppos[{i}]"), pos.to_array());
                u3(gl, p, &format!("u_pcol[{i}]"), *col);
                gl.uniform_1_f32(
                    gl.get_uniform_location(p, &format!("u_prange[{i}]"))
                        .as_ref(),
                    *range,
                );
            }
            let loc = |n: &str| gl.get_uniform_location(p, n);
            let (u_model, u_color, u_hl, u_pat, u_til, u_met, u_rough, u_emi, u_unlit, u_has) = (
                loc("u_model"),
                loc("u_color"),
                loc("u_highlight"),
                loc("u_pattern"),
                loc("u_tiling"),
                loc("u_metallic"),
                loc("u_roughness"),
                loc("u_emission"),
                loc("u_unlit"),
                loc("u_has_tex"),
            );
            let force_unlit = dl.shading == Shading::Unlit;
            for it in &dl.items {
                let Some((vao, count, model_tex)) = self.geometry(gl, it) else {
                    continue;
                };
                let tex = it
                    .texture
                    .as_ref()
                    .and_then(|t| self.texture(gl, t))
                    .or(model_tex);
                gl.use_program(Some(p));
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, tex);
                gl.uniform_1_i32(u_has.as_ref(), tex.is_some() as i32);
                gl.uniform_matrix_4_f32_slice(u_model.as_ref(), false, &it.model.to_cols_array());
                gl.uniform_3_f32(u_color.as_ref(), it.color[0], it.color[1], it.color[2]);
                gl.uniform_1_f32(u_hl.as_ref(), it.highlight);
                gl.uniform_1_i32(
                    u_pat.as_ref(),
                    match it.pattern {
                        Pattern::None => 0,
                        Pattern::Checker => 1,
                        Pattern::Grid => 2,
                    },
                );
                gl.uniform_1_f32(u_til.as_ref(), it.tiling);
                gl.uniform_1_f32(u_met.as_ref(), it.metallic);
                gl.uniform_1_f32(u_rough.as_ref(), it.roughness);
                gl.uniform_3_f32(
                    u_emi.as_ref(),
                    it.emission[0],
                    it.emission[1],
                    it.emission[2],
                );
                gl.uniform_1_i32(u_unlit.as_ref(), (it.unlit || force_unlit) as i32);
                gl.bind_vertex_array(Some(vao));
                gl.draw_arrays(glow::TRIANGLES, 0, count);
            }
            if dl.shading == Shading::Wireframe {
                gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
            }
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, None);

            // 4. Lines (grid, selection, gizmo helpers).
            if !dl.lines.is_empty() {
                let mut buf: Vec<f32> = Vec::with_capacity(dl.lines.len() * 14);
                for (a, b, c) in &dl.lines {
                    buf.extend_from_slice(&[a.x, a.y, a.z, c[0], c[1], c[2], c[3]]);
                    buf.extend_from_slice(&[b.x, b.y, b.z, c[0], c[1], c[2], c[3]]);
                }
                gl.enable(glow::BLEND);
                gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                gl.depth_mask(false);
                gl.use_program(Some(self.line_prog));
                gl.uniform_matrix_4_f32_slice(
                    gl.get_uniform_location(self.line_prog, "u_vp").as_ref(),
                    false,
                    &vpm.to_cols_array(),
                );
                gl.bind_vertex_array(Some(self.line_vao));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.line_vbo));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&buf), glow::STREAM_DRAW);
                gl.draw_arrays(glow::LINES, 0, (dl.lines.len() * 2) as i32);
                gl.depth_mask(true);
            }

            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
        }
    }
}

unsafe fn u3(gl: &glow::Context, p: glow::Program, name: &str, v: [f32; 3]) {
    gl.uniform_3_f32(gl.get_uniform_location(p, name).as_ref(), v[0], v[1], v[2]);
}
