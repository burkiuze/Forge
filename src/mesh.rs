//! Procedural primitive meshes (unit sized, centered) as interleaved `pos(3) + normal(3) + uv(2)`.

use crate::scene::MeshKind;
use glam::{Vec2, Vec3};
use std::f32::consts::{PI, TAU};

pub const STRIDE: usize = 8;

pub struct MeshData {
    pub verts: Vec<f32>,
}

impl MeshData {
    fn new() -> Self {
        Self { verts: vec![] }
    }
    fn push(&mut self, p: Vec3, n: Vec3, uv: Vec2) {
        self.verts
            .extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z, uv.x, uv.y]);
    }
    pub fn vertex_count(&self) -> i32 {
        (self.verts.len() / STRIDE) as i32
    }
}

pub fn build(kind: MeshKind) -> MeshData {
    match kind {
        MeshKind::None | MeshKind::Model => MeshData::new(),
        MeshKind::Cube => cube(),
        MeshKind::Sphere => sphere(32, 20, 0.5, 0.0),
        MeshKind::Plane => plane(),
        MeshKind::Cylinder => cylinder(36, 0.5, 0.5),
        MeshKind::Cone => cylinder(36, 0.5, 0.0),
        MeshKind::Capsule => sphere(32, 20, 0.3, 0.2),
    }
}

fn cube() -> MeshData {
    let mut m = MeshData::new();
    let h = 0.5;
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ];
    for (n, u, v) in faces {
        let c = n * h;
        let p = |a: f32, b: f32| {
            (
                c + u * a * h + v * b * h,
                Vec2::new(a * 0.5 + 0.5, 1.0 - (b * 0.5 + 0.5)),
            )
        };
        let quad = [
            p(-1.0, -1.0),
            p(1.0, -1.0),
            p(1.0, 1.0),
            p(-1.0, -1.0),
            p(1.0, 1.0),
            p(-1.0, 1.0),
        ];
        for (pos, uv) in quad {
            m.push(pos, n, uv);
        }
    }
    m
}

fn plane() -> MeshData {
    // Subdivided so per-vertex effects (fog, point lights) stay smooth on big floors.
    let mut m = MeshData::new();
    let n = 10;
    let s = 1.0 / n as f32;
    for i in 0..n {
        for j in 0..n {
            let x0 = i as f32 * s - 0.5;
            let z0 = j as f32 * s - 0.5;
            let v = |x: f32, z: f32| (Vec3::new(x, 0.0, z), Vec2::new(x + 0.5, z + 0.5));
            let (a, b, c, d) = (v(x0, z0), v(x0 + s, z0), v(x0 + s, z0 + s), v(x0, z0 + s));
            for (p, uv) in [a, d, c, a, c, b] {
                m.push(p, Vec3::Y, uv);
            }
        }
    }
    m
}

/// UV sphere; with `half_len > 0` it becomes a capsule (total height 1).
fn sphere(seg: usize, rings: usize, r: f32, half_len: f32) -> MeshData {
    let mut m = MeshData::new();
    let point = |i: usize, j: usize| -> (Vec3, Vec3, Vec2) {
        let theta = j as f32 / rings as f32 * PI;
        let phi = i as f32 / seg as f32 * TAU;
        let n = Vec3::new(
            theta.sin() * phi.cos(),
            theta.cos(),
            theta.sin() * phi.sin(),
        );
        let mut p = n * r;
        if half_len > 0.0 {
            p.y += if j <= rings / 2 { half_len } else { -half_len };
        }
        (
            p,
            n,
            Vec2::new(i as f32 / seg as f32, j as f32 / rings as f32),
        )
    };
    for j in 0..rings {
        for i in 0..seg {
            let a = point(i, j);
            let b = point(i + 1, j);
            let c = point(i + 1, j + 1);
            let d = point(i, j + 1);
            for (p, n, uv) in [a, c, b, a, d, c] {
                m.push(p, n, uv);
            }
        }
    }
    m
}

fn cylinder(seg: usize, r_bottom: f32, r_top: f32) -> MeshData {
    let mut m = MeshData::new();
    let h = 0.5;
    let slope = r_bottom - r_top;
    for i in 0..seg {
        let (u0, u1) = (i as f32 / seg as f32, (i + 1) as f32 / seg as f32);
        let a0 = u0 * TAU;
        let a1 = u1 * TAU;
        let dir = |a: f32| Vec3::new(a.cos(), 0.0, a.sin());
        let n0 = (dir(a0) + Vec3::Y * slope).normalize();
        let n1 = (dir(a1) + Vec3::Y * slope).normalize();
        let b0 = dir(a0) * r_bottom - Vec3::Y * h;
        let b1 = dir(a1) * r_bottom - Vec3::Y * h;
        let t0 = dir(a0) * r_top + Vec3::Y * h;
        let t1 = dir(a1) * r_top + Vec3::Y * h;
        for (p, n, uv) in [
            (b0, n0, Vec2::new(u0, 1.0)),
            (t1, n1, Vec2::new(u1, 0.0)),
            (t0, n0, Vec2::new(u0, 0.0)),
            (b0, n0, Vec2::new(u0, 1.0)),
            (b1, n1, Vec2::new(u1, 1.0)),
            (t1, n1, Vec2::new(u1, 0.0)),
        ] {
            m.push(p, n, uv);
        }
        let cap_uv = |p: Vec3| Vec2::new(p.x + 0.5, p.z + 0.5);
        let bc = -Vec3::Y * h;
        for p in [bc, b0, b1] {
            m.push(p, Vec3::NEG_Y, cap_uv(p));
        }
        if r_top > 0.0 {
            let tc = Vec3::Y * h;
            for p in [tc, t1, t0] {
                m.push(p, Vec3::Y, cap_uv(p));
            }
        }
    }
    m
}

/// Triangle count per primitive (for the Stats overlay).
pub fn triangles(kind: MeshKind) -> usize {
    match kind {
        MeshKind::None | MeshKind::Model => 0,
        MeshKind::Cube => 12,
        MeshKind::Plane => 200,
        MeshKind::Sphere | MeshKind::Capsule => 32 * 20 * 2,
        MeshKind::Cylinder => 36 * 4,
        MeshKind::Cone => 36 * 3,
    }
}
