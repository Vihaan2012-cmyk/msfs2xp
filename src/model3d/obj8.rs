//! Writing X-Plane OBJ8 files.
//!
//! OBJ8 allows one base texture per object, while an Asobo model usually has
//! several materials with a texture each. A model is therefore split into one
//! object per base texture; all of them are placed at the same point.
//!
//! Axes are turned 180 degrees about the vertical. In MSFS model files a model
//! faces +Z with +X on its left (glTF convention), so for a placement heading of
//! zero +Z points north and +X west. X-Plane object space has +X east and +Z
//! south, so X and Z are both negated. This is a rotation, not a mirror, so
//! triangle winding is unchanged. (O'Hare's Terminal 3 confirms it: its model
//! lies at +X and -Z of a container placed with heading 180, and the terminal
//! is east and north of that point.) Texture rows also need care: glTF
//! puts V = 0 at the top of the image and X-Plane puts T = 0 at the bottom, so
//! T = 1 - V. The texture converter keeps images in X-Plane's orientation so the
//! same rule holds for every texture format.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::glb::{AlphaMode, Model};

/// How to write one object.
#[derive(Debug, Clone, Default)]
pub struct ObjOptions {
    /// Path of the base texture relative to the object file.
    pub texture: Option<String>,
    /// Uniform scale baked into the vertices (DSF placements cannot scale).
    pub scale: f32,
    /// Height in metres added to every vertex, for objects MSFS places above
    /// the ground (DSF placements always sit on the terrain).
    pub offset_y: f32,
    /// Night texture: X-Plane shows it after dark, like MSFS's day/night
    /// emissive materials.
    pub texture_lit: Option<String>,
    /// Write the model's lights into this object (only one object per model).
    pub lights: bool,
}

/// Group a model's meshes by (base texture, night texture), in a stable order.
/// An object can carry one of each, so glowing materials get their own group
/// and nothing else samples their night texture.
pub fn split_by_texture(model: &Model) -> Vec<(Option<String>, Option<String>, Vec<usize>)> {
    let mut groups: BTreeMap<(Option<String>, Option<String>), Vec<usize>> = BTreeMap::new();
    for (i, mesh) in model.meshes.iter().enumerate() {
        if mesh.indices.is_empty() {
            continue;
        }
        let m = model.materials.get(mesh.material);
        let tex = m.and_then(|m| m.base_color.clone());
        let lit = m.filter(|m| m.emissive_strength > 0.0).and_then(|m| m.emissive.clone());
        groups.entry((tex, lit)).or_default().push(i);
    }
    groups.into_iter().map(|((t, l), v)| (t, l, v)).collect()
}

/// Render the given meshes of a model as one OBJ8 file.
pub fn write_obj8(model: &Model, meshes: &[usize], opts: &ObjOptions) -> String {
    let scale = if opts.scale.is_finite() && opts.scale > 0.0 {
        opts.scale
    } else {
        1.0
    };
    let mut vt = String::new();
    let mut indices: Vec<u32> = Vec::new();
    // (first index, count, material) per mesh, for the command section.
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    let mut base = 0u32;
    for &mi in meshes {
        let Some(mesh) = model.meshes.get(mi) else { continue };
        if mesh.indices.is_empty() {
            continue;
        }
        for v in &mesh.vertices {
            let _ = writeln!(
                vt,
                "VT {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.5} {:.5}",
                // Adding 0.0 turns -0.0 into 0.0 so output stays tidy.
                -v.pos[0] * scale + 0.0,
                v.pos[1] * scale + opts.offset_y,
                -v.pos[2] * scale + 0.0,
                -v.normal[0] + 0.0,
                v.normal[1],
                -v.normal[2] + 0.0,
                v.uv[0],
                1.0 - v.uv[1]
            );
        }
        spans.push((indices.len(), mesh.indices.len(), mesh.material));
        indices.extend(mesh.indices.iter().map(|&i| i + base));
        base += mesh.vertices.len() as u32;
    }

    let mut out = String::with_capacity(vt.len() + indices.len() * 8 + 256);
    out.push_str("I\n800\nOBJ\n\n");
    if let Some(tex) = &opts.texture {
        let _ = writeln!(out, "TEXTURE {tex}");
    }
    if let Some(lit) = &opts.texture_lit {
        let _ = writeln!(out, "TEXTURE_LIT {lit}");
    }
    let _ = writeln!(out, "POINT_COUNTS {} 0 0 {}\n", base, indices.len());
    out.push_str(&vt);
    out.push('\n');
    let mut chunks = indices.chunks_exact(10);
    for c in chunks.by_ref() {
        let _ = writeln!(
            out,
            "IDX10 {} {} {} {} {} {} {} {} {} {}",
            c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7], c[8], c[9]
        );
    }
    for i in chunks.remainder() {
        let _ = writeln!(out, "IDX {i}");
    }
    out.push('\n');

    // Emit state changes only when they differ from the previous span.
    let (mut cull, mut blend, mut offset): (Option<bool>, Option<String>, Option<u8>) = (None, None, None);
    for (first, count, mat) in spans {
        let m = model.materials.get(mat).cloned().unwrap_or_default();
        let want_cull = !m.double_sided;
        if cull != Some(want_cull) {
            out.push_str(if want_cull { "ATTR_cull\n" } else { "ATTR_no_cull\n" });
            cull = Some(want_cull);
        }
        let want_blend = match m.alpha {
            AlphaMode::Blend => "ATTR_blend".to_string(),
            AlphaMode::Mask => format!("ATTR_no_blend {:.2}", m.alpha_cutoff.clamp(0.0, 1.0)),
            // glTF OPAQUE ignores alpha. MSFS opaque albedo textures often hold
            // unrelated data in alpha, and a 0.5 cutoff would punch holes in walls.
            AlphaMode::Opaque => "ATTR_no_blend 0.00".to_string(),
        };
        if blend.as_deref() != Some(want_blend.as_str()) {
            let _ = writeln!(out, "{want_blend}");
            blend = Some(want_blend);
        }
        // Decals sit a hair above other geometry; polygon offset stops z-fighting.
        let want_offset = if m.decal { 2 } else { 0 };
        if offset != Some(want_offset) {
            let _ = writeln!(out, "ATTR_poly_os {want_offset}");
            offset = Some(want_offset);
        }
        let _ = writeln!(out, "TRIS {first} {count}");
    }
    if opts.lights {
        for l in &model.lights {
            // Same axis turn as the vertices; the height offset lifts lights too.
            let (x, y, z) = (
                -l.pos[0] * scale + 0.0,
                l.pos[1] * scale + opts.offset_y,
                -l.pos[2] * scale + 0.0,
            );
            let (dx, dy, dz) = (-l.dir[0] + 0.0, l.dir[1] + 0.0, -l.dir[2] + 0.0);
            // X-Plane's WIDTH is the cosine of half the cone (Laminar's 120 degree
            // ramp floods use 0.5).
            let width = ((l.cone_deg.clamp(0.0, 360.0) as f64) / 2.0).to_radians().cos().max(0.0);
            let [r, g, b] = l.color;
            if l.spill && !l.flashing {
                let _ = writeln!(
                    out,
                    "LIGHT_PARAM spot_params_sp_pm {x:.3} {y:.3} {z:.3} {r:.3} {g:.3} {b:.3} 1 {:.0}cd {dx:.4} {dy:.4} {dz:.4} {width:.4}",
                    l.intensity.max(1.0)
                );
            }
            let _ = writeln!(
                out,
                "LIGHT_PARAM spot_params_bb_pm {x:.3} {y:.3} {z:.3} {r:.3} {g:.3} {b:.3} {:.0}cd {dx:.4} {dy:.4} {dz:.4} {width:.4}",
                (l.intensity / 6.0).max(100.0)
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model3d::glb::{LightPoint, Material, Mesh, Vertex};

    fn model(n_tris: usize, material: Material) -> Model {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for t in 0..n_tris {
            let base = vertices.len() as u32;
            for k in 0..3 {
                vertices.push(Vertex {
                    pos: [t as f32, k as f32, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.25],
                });
            }
            indices.extend([base, base + 1, base + 2]);
        }
        Model {
            meshes: vec![Mesh {
                name: "m".into(),
                material: 0,
                vertices,
                indices,
            }],
            materials: vec![material],
            warnings: vec![],
            lights: vec![],
        }
    }

    #[test]
    fn models_are_turned_to_xplane_axes_and_lifted() {
        // Two triangles: the second sits at x = 1 in model space.
        let m = model(
            2,
            Material {
                base_color: Some("a.dds".into()),
                ..Default::default()
            },
        );
        let s = write_obj8(
            &m,
            &[0],
            &ObjOptions {
                texture: None,
                scale: 1.0,
                offset_y: 5.0,
                texture_lit: None,
                lights: false,
            },
        );
        // Model +X becomes object -X, and the height offset lifts every vertex.
        assert!(s.contains("VT -1.0000 6.0000 0.0000 0.0000 0.0000 -1.0000"), "{s}");
        assert!(!s.contains("VT -0.0000"), "no negative zeros: {s}");
    }

    #[test]
    fn writes_night_texture_and_lights() {
        let mut m = model(
            1,
            Material {
                base_color: Some("a.dds".into()),
                ..Default::default()
            },
        );
        m.lights.push(LightPoint {
            pos: [1.0, 10.0, 2.0],
            dir: [0.0, -1.0, 1.0],
            color: [1.0, 1.0, 1.0],
            intensity: 7500.0,
            cone_deg: 120.0,
            spill: true,
            flashing: false,
        });
        let s = write_obj8(
            &m,
            &[0],
            &ObjOptions {
                texture: Some("t/a.dds".into()),
                scale: 1.0,
                offset_y: 0.0,
                texture_lit: Some("t/a_lit.dds".into()),
                lights: true,
            },
        );
        assert!(s.contains("TEXTURE t/a.dds\nTEXTURE_LIT t/a_lit.dds\nPOINT_COUNTS"), "{s}");
        assert!(
            s.contains("LIGHT_PARAM spot_params_sp_pm -1.000 10.000 -2.000 1.000 1.000 1.000 1 7500cd 0.0000 -1.0000 -1.0000 0.5000"),
            "{s}"
        );
        assert!(s.contains("LIGHT_PARAM spot_params_bb_pm -1.000 10.000 -2.000"), "{s}");
    }

    #[test]
    fn glowing_materials_get_their_own_object() {
        let mut m = model(
            1,
            Material {
                base_color: Some("a.dds".into()),
                ..Default::default()
            },
        );
        m.materials.push(Material {
            base_color: Some("a.dds".into()),
            emissive: Some("a.dds".into()),
            emissive_strength: 150.0,
            ..Default::default()
        });
        let mut second = m.meshes[0].clone();
        second.material = 1;
        m.meshes.push(second);
        let groups = split_by_texture(&m);
        assert_eq!(groups.len(), 2, "same base texture, different night texture");
        assert!(groups.iter().any(|g| g.1.as_deref() == Some("a.dds") && g.2 == vec![1]));
        assert!(groups.iter().any(|g| g.1.is_none() && g.2 == vec![0]));
    }

    #[test]
    fn writes_the_obj8_structure() {
        let m = model(
            1,
            Material {
                base_color: Some("a.dds".into()),
                ..Default::default()
            },
        );
        let s = write_obj8(
            &m,
            &[0],
            &ObjOptions {
                texture: Some("textures/a.dds".into()),
                scale: 2.0,
                offset_y: 0.0,
                texture_lit: None,
                lights: false,
            },
        );
        assert!(s.starts_with("I\n800\nOBJ\n\nTEXTURE textures/a.dds\nPOINT_COUNTS 3 0 0 3\n"));
        assert!(s.contains("VT 0.0000 2.0000 0.0000"), "scale is baked in:\n{s}");
        assert!(s.contains(" 0.00000 0.75000\n"), "V is flipped to T");
        assert!(s.contains("ATTR_cull\nATTR_no_blend 0.00\nATTR_poly_os 0\nTRIS 0 3\n"));
    }

    #[test]
    fn indices_come_in_tens_then_singles() {
        let m = model(9, Material::default()); // 27 indices
        let s = write_obj8(&m, &[0], &ObjOptions::default());
        assert_eq!(s.lines().filter(|l| l.starts_with("IDX10 ")).count(), 2);
        assert_eq!(s.lines().filter(|l| l.starts_with("IDX ")).count(), 7);
    }

    #[test]
    fn material_state_maps_to_attributes() {
        let m = model(
            1,
            Material {
                double_sided: true,
                alpha: AlphaMode::Mask,
                alpha_cutoff: 0.3,
                decal: true,
                ..Default::default()
            },
        );
        let s = write_obj8(&m, &[0], &ObjOptions::default());
        assert!(s.contains("ATTR_no_cull\nATTR_no_blend 0.30\nATTR_poly_os 2\n"), "{s}");
    }

    #[test]
    fn meshes_split_by_texture() {
        let mut m = model(1, Material::default());
        m.materials = vec![
            Material {
                base_color: Some("b.dds".into()),
                ..Default::default()
            },
            Material {
                base_color: Some("a.dds".into()),
                ..Default::default()
            },
        ];
        let mut second = m.meshes[0].clone();
        second.material = 1;
        m.meshes.push(second);
        let groups = split_by_texture(&m);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0.as_deref(), Some("a.dds"));
        assert_eq!(groups[0].2, vec![1]);
    }

    #[test]
    fn a_real_glb_round_trips_to_obj8() {
        let glb = crate::model3d::glb::tests::asobo_triangle("");
        let model = crate::model3d::load_glb(&glb).unwrap();
        let groups = split_by_texture(&model);
        let s = write_obj8(&model, &groups[0].2, &ObjOptions::default());
        assert!(s.contains("POINT_COUNTS 3 0 0 3"));
        assert!(s.contains("ATTR_no_cull"));
    }
}
