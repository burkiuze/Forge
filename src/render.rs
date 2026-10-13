//! GPU viewport renderer (OpenGL via glow) + cameras, projection and picking.

use crate::mesh;
use crate::scene::{EntityId, MeshKind, Scene};
use eframe::glow::{self, HasContext};
use glam::{Mat4, Vec3, Vec4};
use std::collections::HashMap;

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
        Self { target: Vec3::new(0.0, 0.5, 0.0), yaw: 0.6, pitch: 0.5, distance: 16.0, fov: 55.0 }
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
        self.target += (fwd * local.z + right * local.x + Vec3::Y * local.y) * (self.distance * 0.04).max(0.08);
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
        proj: Mat4::perspective_rh_gl(cam.fov.clamp(10.0, 150.0).to_radians(), aspect, 0.05, 2000.0),
        eye: pos,
    })
}

pub fn main_camera(scene: &Scene) -> Option<EntityId> {
    scene.entities.iter().find(|e| e.camera.is_some() && e.active).map(|e| e.id)
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
}

pub struct DrawList {
    pub cam: CameraView,
    pub sun_dir: Vec3,
    pub ambient: f32,
    pub sky_top: [f32; 3],
    pub sky_bottom: [f32; 3],
    pub items: Vec<DrawItem>,
    /// Depth-tested lines: (a, b, color).
    pub lines: Vec<(Vec3, Vec3, [f32; 4])>,
}

impl DrawList {
    pub fn from_scene(scene: &Scene, cam: CameraView, selected: Option<EntityId>, grid: bool) -> Self {
        let env = &scene.environment;
        let mut items = Vec::with_capacity(scene.entities.len());
        for e in &scene.entities {
            if e.mesh == MeshKind::None || !scene.is_active(e.id) {
                continue;
            }
            items.push(DrawItem {
                mesh: e.mesh,
                model: scene.world_matrix(e.id),
                color: e.color,
                highlight: if Some(e.id) == selected { 1.0 } else { 0.0 },
            });
        }
        let mut lines = vec![];
        if grid {
            let n = 30;
            for i in -n..=n {
                let a = if i == 0 { 0.35 } else if i % 5 == 0 { 0.16 } else { 0.07 };
                let f = i as f32;
                let nn = n as f32;
                let cx = if i == 0 { [0.35, 0.45, 0.95, a] } else { [1.0, 1.0, 1.0, a] };
                let cz = if i == 0 { [0.95, 0.35, 0.3, a] } else { [1.0, 1.0, 1.0, a] };
                lines.push((Vec3::new(f, 0.002, -nn), Vec3::new(f, 0.002, nn), cx));
                lines.push((Vec3::new(-nn, 0.002, f), Vec3::new(nn, 0.002, f), cz));
            }
        }
        if let Some(sel) = selected.and_then(|s| scene.get(s)) {
            let m = scene.world_matrix(sel.id);
            let c = [1.0, 0.55, 0.15, 1.0];
            let corners: Vec<Vec3> = (0..8)
                .map(|i| {
                    let p = Vec3::new(
                        if i & 1 == 0 { -0.5 } else { 0.5 },
                        if i & 2 == 0 { -0.5 } else { 0.5 },
                        if i & 4 == 0 { -0.5 } else { 0.5 },
                    );
                    let p = if sel.mesh == MeshKind::Plane { p * Vec3::new(1.0, 0.0, 1.0) } else { p };
                    m.transform_point3(p * 1.01)
                })
                .collect();
            for (a, b) in [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)] {
                lines.push((corners[a], corners[b], c));
            }
        }
        Self {
            cam,
            sun_dir: scene.sun_dir(),
            ambient: env.ambient,
            sky_top: env.sky_top,
            sky_bottom: env.sky_bottom,
            items,
            lines,
        }
    }
}

// ---------------------------------------------------------------- GL renderer

const MESH_VS: &str = r#"
layout(location=0) in vec3 a_pos;
layout(location=1) in vec3 a_nrm;
uniform mat4 u_vp;
uniform mat4 u_model;
out vec3 v_world;
out vec3 v_nrm;
out vec3 v_local;
void main() {
    vec4 w = u_model * vec4(a_pos, 1.0);
    v_world = w.xyz;
    v_local = a_pos;
    v_nrm = mat3(transpose(inverse(u_model))) * a_nrm;
    gl_Position = u_vp * w;
}
"#;

const MESH_FS: &str = r#"
in vec3 v_world;
in vec3 v_nrm;
in vec3 v_local;
uniform vec3 u_color;
uniform vec3 u_sun;
uniform vec3 u_eye;
uniform vec3 u_sky_top;
uniform vec3 u_sky_bottom;
uniform float u_ambient;
uniform float u_highlight;
uniform float u_checker;
out vec4 o_color;
void main() {
    vec3 n = normalize(v_nrm);
    if (!gl_FrontFacing) n = -n;
    vec3 l = normalize(-u_sun);
    float diff = max(dot(n, l), 0.0);
    vec3 v = normalize(u_eye - v_world);
    vec3 h = normalize(l + v);
    float spec = pow(max(dot(n, h), 0.0), 48.0) * 0.25;
    vec3 hemi = mix(u_sky_bottom, u_sky_top, n.y * 0.5 + 0.5);
    vec3 base = u_color;
    if (u_checker > 0.5) {
        vec2 c = floor(v_world.xz);
        float k = mod(c.x + c.y, 2.0);
        base *= mix(0.9, 1.05, k);
    }
    vec3 col = base * (diff * 0.85 + hemi * u_ambient * 1.6) + vec3(spec);
    float rim = pow(1.0 - max(dot(n, v), 0.0), 3.0);
    col += vec3(1.0, 0.45, 0.1) * rim * u_highlight * 0.9;
    float dist = length(u_eye - v_world);
    float fog = clamp((dist - 60.0) / 240.0, 0.0, 1.0);
    col = mix(col, u_sky_bottom, fog);
    col = col / (col + vec3(0.85)) * 1.6;  // soft tonemap
    o_color = vec4(pow(col, vec3(1.0 / 1.15)), 1.0);
}
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
out float v_t;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_t = p.y * 0.5;
    gl_Position = vec4(p * 2.0 - 1.0, 0.999, 1.0);
}
"#;

const SKY_FS: &str = r#"
in float v_t;
uniform vec3 u_sky_top;
uniform vec3 u_sky_bottom;
out vec4 o_color;
void main() { o_color = vec4(mix(u_sky_bottom, u_sky_top, clamp(v_t * 1.3, 0.0, 1.0)), 1.0); }
"#;

struct GpuMesh {
    vao: glow::VertexArray,
    _vbo: glow::Buffer,
    count: i32,
}

pub struct GlRenderer {
    mesh_prog: glow::Program,
    line_prog: glow::Program,
    sky_prog: glow::Program,
    sky_vao: glow::VertexArray,
    meshes: HashMap<MeshKind, GpuMesh>,
    line_vao: glow::VertexArray,
    line_vbo: glow::Buffer,
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
            let line_prog = compile(gl, LINE_VS, LINE_FS)?;
            let sky_prog = compile(gl, SKY_VS, SKY_FS)?;
            let sky_vao = gl.create_vertex_array()?;
            let mut meshes = HashMap::new();
            for kind in MeshKind::ALL {
                if kind == MeshKind::None {
                    continue;
                }
                let data = mesh::build(kind);
                let vao = gl.create_vertex_array()?;
                let vbo = gl.create_buffer()?;
                gl.bind_vertex_array(Some(vao));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&data.verts), glow::STATIC_DRAW);
                gl.enable_vertex_attrib_array(0);
                gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 24, 0);
                gl.enable_vertex_attrib_array(1);
                gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 24, 12);
                meshes.insert(kind, GpuMesh { vao, _vbo: vbo, count: data.vertex_count() });
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
            Ok(Self { mesh_prog, line_prog, sky_prog, sky_vao, meshes, line_vao, line_vbo })
        }
    }

    pub fn paint(&self, gl: &glow::Context, dl: &DrawList, info: &egui::PaintCallbackInfo) {
        let vp = info.viewport_in_pixels();
        let clip = info.clip_rect_in_pixels();
        unsafe {
            gl.viewport(vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px);
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(clip.left_px, clip.from_bottom_px, clip.width_px, clip.height_px);
            gl.depth_mask(true);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::BLEND);

            // Sky gradient.
            gl.disable(glow::DEPTH_TEST);
            gl.use_program(Some(self.sky_prog));
            self.u3(gl, self.sky_prog, "u_sky_top", dl.sky_top);
            self.u3(gl, self.sky_prog, "u_sky_bottom", dl.sky_bottom);
            gl.bind_vertex_array(Some(self.sky_vao));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            // Meshes.
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            let p = self.mesh_prog;
            gl.use_program(Some(p));
            let vpm = dl.cam.view_proj().to_cols_array();
            gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(p, "u_vp").as_ref(), false, &vpm);
            self.u3(gl, p, "u_sun", dl.sun_dir.to_array());
            self.u3(gl, p, "u_eye", dl.cam.eye.to_array());
            self.u3(gl, p, "u_sky_top", dl.sky_top);
            self.u3(gl, p, "u_sky_bottom", dl.sky_bottom);
            gl.uniform_1_f32(gl.get_uniform_location(p, "u_ambient").as_ref(), dl.ambient);
            let u_model = gl.get_uniform_location(p, "u_model");
            let u_color = gl.get_uniform_location(p, "u_color");
            let u_hl = gl.get_uniform_location(p, "u_highlight");
            let u_chk = gl.get_uniform_location(p, "u_checker");
            for it in &dl.items {
                let Some(m) = self.meshes.get(&it.mesh) else { continue };
                gl.uniform_matrix_4_f32_slice(u_model.as_ref(), false, &it.model.to_cols_array());
                gl.uniform_3_f32(u_color.as_ref(), it.color[0], it.color[1], it.color[2]);
                gl.uniform_1_f32(u_hl.as_ref(), it.highlight);
                gl.uniform_1_f32(u_chk.as_ref(), if it.mesh == MeshKind::Plane { 1.0 } else { 0.0 });
                gl.bind_vertex_array(Some(m.vao));
                gl.draw_arrays(glow::TRIANGLES, 0, m.count);
            }

            // Lines (grid, selection).
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
                gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(self.line_prog, "u_vp").as_ref(), false, &vpm);
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

    unsafe fn u3(&self, gl: &glow::Context, p: glow::Program, name: &str, v: [f32; 3]) {
        gl.uniform_3_f32(gl.get_uniform_location(p, name).as_ref(), v[0], v[1], v[2]);
    }
}
