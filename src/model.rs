//! glTF / GLB model import.
//!
//! Meshes are flattened (node transforms applied) and normalized into the engine's unit cube
//! (-0.5..0.5 on every axis); the original size is returned as `extents` and becomes the
//! entity's scale. That way picking, physics and gizmos treat models like any primitive.

use glam::{Mat4, Vec3};
use std::path::Path;

pub struct ModelData {
    /// Interleaved pos(3) + normal(3) + uv(2), see `mesh::STRIDE`.
    pub verts: Vec<f32>,
    /// Base-color texture (RGBA8).
    pub texture: Option<(u32, u32, Vec<u8>)>,
    pub base_color: [f32; 3],
    /// Original model size in its own units.
    pub extents: Vec3,
}

fn to_rgba(img: &gltf::image::Data) -> Option<Vec<u8>> {
    use gltf::image::Format;
    let px = &img.pixels;
    Some(match img.format {
        Format::R8G8B8A8 => px.clone(),
        Format::R8G8B8 => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        Format::R8G8 => px
            .chunks(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        Format::R8 => px.iter().flat_map(|c| [*c, *c, *c, 255]).collect(),
        _ => return None,
    })
}

pub fn load(path: &Path) -> Result<ModelData, String> {
    let (doc, buffers, images) =
        gltf::import(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut raw: Vec<(Vec3, Vec3, [f32; 2])> = vec![];
    let mut texture = None;
    let mut base_color = [1.0, 1.0, 1.0];

    fn visit(
        node: gltf::Node,
        parent: Mat4,
        buffers: &[gltf::buffer::Data],
        images: &[gltf::image::Data],
        raw: &mut Vec<(Vec3, Vec3, [f32; 2])>,
        texture: &mut Option<(u32, u32, Vec<u8>)>,
        base_color: &mut [f32; 3],
    ) {
        let m = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        let nm = m.inverse().transpose();
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                if prim.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = prim.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
                let Some(pos) = reader.read_positions() else {
                    continue;
                };
                let pos: Vec<Vec3> = pos.map(|p| m.transform_point3(Vec3::from(p))).collect();
                let nrm: Vec<Vec3> = reader
                    .read_normals()
                    .map(|n| {
                        n.map(|n| nm.transform_vector3(Vec3::from(n)).normalize_or_zero())
                            .collect()
                    })
                    .unwrap_or_else(|| vec![Vec3::ZERO; pos.len()]);
                let uv: Vec<[f32; 2]> = reader
                    .read_tex_coords(0)
                    .map(|t| t.into_f32().collect())
                    .unwrap_or_else(|| vec![[0.0, 0.0]; pos.len()]);
                let idx: Vec<u32> = reader
                    .read_indices()
                    .map(|i| i.into_u32().collect())
                    .unwrap_or_else(|| (0..pos.len() as u32).collect());
                for tri in idx.chunks_exact(3) {
                    let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
                    if a >= pos.len() || b >= pos.len() || c >= pos.len() {
                        continue;
                    }
                    let flat = (pos[b] - pos[a]).cross(pos[c] - pos[a]).normalize_or_zero();
                    for i in [a, b, c] {
                        let n = if nrm[i] == Vec3::ZERO { flat } else { nrm[i] };
                        raw.push((pos[i], n, uv[i]));
                    }
                }
                let pbr = prim.material().pbr_metallic_roughness();
                if texture.is_none() {
                    let f = pbr.base_color_factor();
                    *base_color = [f[0], f[1], f[2]];
                    if let Some(info) = pbr.base_color_texture() {
                        if let Some(img) = images.get(info.texture().source().index()) {
                            if let Some(rgba) = to_rgba(img) {
                                *texture = Some((img.width, img.height, rgba));
                            }
                        }
                    }
                }
            }
        }
        for child in node.children() {
            visit(child, m, buffers, images, raw, texture, base_color);
        }
    }

    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or("model has no scene")?;
    for node in scene.nodes() {
        visit(
            node,
            Mat4::IDENTITY,
            &buffers,
            &images,
            &mut raw,
            &mut texture,
            &mut base_color,
        );
    }
    if raw.is_empty() {
        return Err("model contains no triangles".into());
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for (p, _, _) in &raw {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let size = hi - lo;
    let ext = Vec3::new(size.x.max(1e-4), size.y.max(1e-4), size.z.max(1e-4));
    let center = (lo + hi) * 0.5;
    let mut verts = Vec::with_capacity(raw.len() * 8);
    for (p, n, uv) in &raw {
        let q = (*p - center) / ext;
        // Normals must survive the entity's scale (= ext) being applied back in the shader.
        let n = (*n * ext).normalize_or_zero();
        verts.extend_from_slice(&[q.x, q.y, q.z, n.x, n.y, n.z, uv[0], uv[1]]);
    }
    Ok(ModelData {
        verts,
        texture,
        base_color,
        extents: ext,
    })
}
