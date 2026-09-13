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
    /// Normal map in X-Plane's NORMAL_METALNESS layout.
    pub texture_normal: Option<String>,
}

/// The textures one object carries: base, night and, when normal maps are
/// converted, the normal map with its metal/roughness companion.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextureKey {
    pub base: Option<String>,
    pub lit: Option<String>,
    pub normal: Option<String>,
    pub metal_rough: Option<String>,
}

/// Group a model's meshes by the textures they need, in a stable order. An
/// object can carry one of each, so glowing materials (and, with `normals`,
/// materials with their own normal map) get their own group.
pub fn split_by_texture(model: &Model, normals: bool) -> Vec<(TextureKey, Vec<usize>)> {
    let mut groups: BTreeMap<TextureKey, Vec<usize>> = BTreeMap::new();
    for (i, mesh) in model.meshes.iter().enumerate() {
        if mesh.indices.is_empty() {
            continue;
        }
        let m = model.materials.get(mesh.material);
        let normal = m.filter(|_| normals).and_then(|m| m.normal.clone());
        let key = TextureKey {
            base: m.and_then(|m| m.base_color.clone()),
            lit: m.filter(|m| m.emissive_strength > 0.0).and_then(|m| m.emissive.clone()),
            metal_rough: m.filter(|_| normal.is_some()).and_then(|m| m.metal_rough.clone()),
            normal,
        };
        groups.entry(key).or_default().push(i);
    }
    groups.into_iter().collect()
}

/// One level of detail: some meshes of a model, drawn when the viewer is
/// between `near` and `far` metres away.
#[derive(Debug, Clone)]
pub struct LodPart<'a> {
    pub model: &'a Model,
    pub meshes: Vec<usize>,
    pub near: f32,
    /// Infinite means no distance limit.
    pub far: f32,
}

/// Render the given meshes of a model as one OBJ8 file.
pub fn write_obj8(model: &Model, meshes: &[usize], opts: &ObjOptions) -> String {
    write_obj8_lods(
        &[LodPart {
            model,
            meshes: meshes.to_vec(),
            near: 0.0,
            far: f32::INFINITY,
        }],
        opts,
    )
}

/// Render levels of detail as one OBJ8 file. X-Plane draws only the part whose
/// range holds the viewer's distance, and nothing beyond the last part. The
/// lights (the first part's model's) are repeated in every part so they show
/// whichever part is drawn.
pub fn write_obj8_lods(parts: &[LodPart], opts: &ObjOptions) -> String {
    let scale = if opts.scale.is_finite() && opts.scale > 0.0 {
        opts.scale
    } else {
        1.0
    };
    let ranged = parts.iter().any(|p| p.far.is_finite());
    let mut vt = String::new();
    let mut indices: Vec<u32> = Vec::new();
    // Per part, (first index, count, material) per mesh, for the command section.
    let mut part_spans: Vec<Vec<(usize, usize, usize)>> = Vec::new();
    let mut base = 0u32;
    for part in parts {
        let mut spans = Vec::new();
        for &mi in &part.meshes {
            let Some(mesh) = part.model.meshes.get(mi) else { continue };
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
        part_spans.push(spans);
    }

    let mut out = String::with_capacity(vt.len() + indices.len() * 8 + 256);
    out.push_str("I\n800\nOBJ\n\n");
    if let Some(tex) = &opts.texture {
        let _ = writeln!(out, "TEXTURE {tex}");
    }
    if let Some(lit) = &opts.texture_lit {
        let _ = writeln!(out, "TEXTURE_LIT {lit}");
    }
    if let Some(normal) = &opts.texture_normal {
        let _ = writeln!(out, "TEXTURE_NORMAL {normal}");
        out.push_str("NORMAL_METALNESS\n");
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

    for (part, spans) in parts.iter().zip(&part_spans) {
        if ranged {
            let far = if part.far.is_finite() { part.far } else { 100_000.0 };
            let _ = writeln!(out, "ATTR_LOD {:.0} {:.0}", part.near, far);
        }
        // Emit state changes only when they differ from the previous span;
        // each level of detail starts afresh.
        let (mut cull, mut blend, mut offset): (Option<bool>, Option<String>, Option<u8>) = (None, None, None);
        for &(first, count, mat) in spans {
            let m = part.model.materials.get(mat).cloned().unwrap_or_default();
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
            if let Some(first) = parts.first() {
                write_lights(&mut out, first.model, scale, opts.offset_y);
            }
        }
    }
    out
}

fn write_lights(out: &mut String, model: &Model, scale: f32, offset_y: f32) {
    for l in &model.lights {
        // Same axis turn as the vertices; the height offset lifts lights too.
        let (x, y, z) = (
            -l.pos[0] * scale + 0.0,
            l.pos[1] * scale + offset_y,
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
                texture_normal: None,
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
                texture_normal: Some("t/a_nm.png".into()),
            },
        );
        assert!(
            s.contains("TEXTURE t/a.dds\nTEXTURE_LIT t/a_lit.dds\nTEXTURE_NORMAL t/a_nm.png\nNORMAL_METALNESS\nPOINT_COUNTS"),
            "{s}"
        );
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
        let groups = split_by_texture(&m, false);
        assert_eq!(groups.len(), 2, "same base texture, different night texture");
        assert!(groups.iter().any(|g| g.0.lit.as_deref() == Some("a.dds") && g.1 == vec![1]));
        assert!(groups.iter().any(|g| g.0.lit.is_none() && g.1 == vec![0]));
    }

    #[test]
    fn levels_of_detail_become_distance_ranges() {
        let mut near = model(2, Material::default());
        near.lights.push(LightPoint {
            pos: [0.0, 10.0, 0.0],
            dir: [0.0, -1.0, 0.0],
            color: [1.0, 1.0, 1.0],
            intensity: 600.0,
            cone_deg: 120.0,
            spill: true,
            flashing: false,
        });
        let far = model(1, Material::default());
        let s = write_obj8_lods(
            &[
                LodPart {
                    model: &near,
                    meshes: vec![0],
                    near: 0.0,
                    far: 40.0,
                },
                LodPart {
                    model: &far,
                    meshes: vec![0],
                    near: 40.0,
                    far: 400.0,
                },
            ],
            &ObjOptions {
                lights: true,
                ..Default::default()
            },
        );
        assert!(s.contains("POINT_COUNTS 9 0 0 9"), "{s}");
        let near_at = s.find("ATTR_LOD 0 40\n").expect("near range");
        let far_at = s.find("ATTR_LOD 40 400\n").expect("far range");
        let near_tris = s.find("TRIS 0 6\n").expect("near triangles");
        let far_tris = s.find("TRIS 6 3\n").expect("far triangles");
        assert!(near_at < near_tris && near_tris < far_at && far_at < far_tris, "{s}");
        assert_eq!(s.matches("spot_params_bb_pm").count(), 2, "lights repeat per level");
        assert!(!write_obj8(&near, &[0], &ObjOptions::default()).contains("ATTR_LOD"));
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
                texture_normal: None,
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
        let groups = split_by_texture(&m, false);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0.base.as_deref(), Some("a.dds"));
        assert_eq!(groups[0].1, vec![1]);
        // With normal maps on, a different normal map splits the same base texture.
        m.materials[1].normal = Some("n.dds".into());
        m.materials[0].base_color = Some("a.dds".into());
        let groups = split_by_texture(&m, true);
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().any(|g| g.0.normal.as_deref() == Some("n.dds")));
        assert_eq!(split_by_texture(&m, false).len(), 1, "without normal maps they share an object");
    }

    #[test]
    fn a_real_glb_round_trips_to_obj8() {
        let glb = crate::model3d::glb::tests::asobo_triangle("");
        let model = crate::model3d::load_glb(&glb).unwrap();
        let groups = split_by_texture(&model, false);
        let s = write_obj8(&model, &groups[0].1, &ObjOptions::default());
        assert!(s.contains("POINT_COUNTS 3 0 0 3"));
        assert!(s.contains("ATTR_no_cull"));
    }
}
