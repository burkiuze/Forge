//! Procedural primitive meshes (unit sized, centered) as interleaved `pos(3) + normal(3)`.

use crate::scene::MeshKind;
use glam::Vec3;
use std::f32::consts::{PI, TAU};

pub struct MeshData {
    pub verts: Vec<f32>,
}

impl MeshData {
    fn new() -> Self {
        Self { verts: vec![] }
    }
    fn push(&mut self, p: Vec3, n: Vec3) {
        self.verts.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z]);
    }
    fn tri_flat(&mut self, a: Vec3, b: Vec3, c: Vec3) {
        let n = (b - a).cross(c - a).normalize_or_zero();
        self.push(a, n);
        self.push(b, n);
        self.push(c, n);
    }
    pub fn vertex_count(&self) -> i32 {
        (self.verts.len() / 6) as i32
    }
}

pub fn build(kind: MeshKind) -> MeshData {
    match kind {
        MeshKind::None => MeshData::new(),
        MeshKind::Cube => cube(),
        MeshKind::Sphere => sphere(24, 16, 0.5, 0.0),
        MeshKind::Plane => plane(),
        MeshKind::Cylinder => cylinder(32, 0.5, 0.5),
        MeshKind::Cone => cylinder(32, 0.5, 0.0),
        MeshKind::Capsule => sphere(24, 16, 0.3, 0.2),
    }
}

fn cube() -> MeshData {
    let mut m = MeshData::new();
    let h = 0.5;
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::Z, Vec3::NEG_X),
        (Vec3::Z, Vec3::Y, Vec3::NEG_X),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];
    for (n, u, v) in faces {
        let c = n * h;
        let p = |a: f32, b: f32| c + u * a * h + v * b * h;
        let (a, b, cc, d) = (p(-1.0, -1.0), p(-1.0, 1.0), p(1.0, 1.0), p(1.0, -1.0));
        // Wind counter-clockwise when seen from the outside.
        for q in [[a, cc, b], [a, d, cc]] {
            let fn_ = (q[1] - q[0]).cross(q[2] - q[0]);
            let q = if fn_.dot(n) < 0.0 { [q[0], q[2], q[1]] } else { q };
            for p in q {
                m.push(p, n);
            }
        }
    }
    m
}

fn plane() -> MeshData {
    // Subdivided so vertex fog/lighting stays smooth on large floors.
    let mut m = MeshData::new();
    let n = 8;
    for i in 0..n {
        for j in 0..n {
            let x0 = i as f32 / n as f32 - 0.5;
            let z0 = j as f32 / n as f32 - 0.5;
            let s = 1.0 / n as f32;
            let a = Vec3::new(x0, 0.0, z0);
            let b = Vec3::new(x0 + s, 0.0, z0);
            let c = Vec3::new(x0 + s, 0.0, z0 + s);
            let d = Vec3::new(x0, 0.0, z0 + s);
            for p in [a, d, c, a, c, b] {
                m.push(p, Vec3::Y);
            }
        }
    }
    m
}

/// UV sphere; with `half_len > 0` it becomes a capsule (total height 1).
fn sphere(seg: usize, rings: usize, r: f32, half_len: f32) -> MeshData {
    let mut m = MeshData::new();
    let point = |i: usize, j: usize| -> (Vec3, Vec3) {
        let theta = j as f32 / rings as f32 * PI;
        let phi = i as f32 / seg as f32 * TAU;
        let n = Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin());
        let mut p = n * r;
        if half_len > 0.0 {
            p.y += if j <= rings / 2 { half_len } else { -half_len };
        }
        (p, n)
    };
    for j in 0..rings {
        for i in 0..seg {
            let a = point(i, j);
            let b = point(i + 1, j);
            let c = point(i + 1, j + 1);
            let d = point(i, j + 1);
            for (p, n) in [a, b, c, a, c, d] {
                m.push(p, n);
            }
        }
    }
    m
}

fn cylinder(seg: usize, r_bottom: f32, r_top: f32) -> MeshData {
    let mut m = MeshData::new();
    let h = 0.5;
    let slope = (r_bottom - r_top) / 1.0;
    for i in 0..seg {
        let a0 = i as f32 / seg as f32 * TAU;
        let a1 = (i + 1) as f32 / seg as f32 * TAU;
        let dir = |a: f32| Vec3::new(a.cos(), 0.0, a.sin());
        let n0 = (dir(a0) + Vec3::Y * slope).normalize();
        let n1 = (dir(a1) + Vec3::Y * slope).normalize();
        let b0 = dir(a0) * r_bottom - Vec3::Y * h;
        let b1 = dir(a1) * r_bottom - Vec3::Y * h;
        let t0 = dir(a0) * r_top + Vec3::Y * h;
        let t1 = dir(a1) * r_top + Vec3::Y * h;
        for (p, n) in [(b0, n0), (t0, n0), (t1, n1), (b0, n0), (t1, n1), (b1, n1)] {
            m.push(p, n);
        }
        m.tri_flat(-Vec3::Y * h, b0, b1);
        if r_top > 0.0 {
            m.tri_flat(Vec3::Y * h, t1, t0);
        }
    }
    m
}
