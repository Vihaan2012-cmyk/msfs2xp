//! Development helper: which glTF features a package's models use that could
//! move textures around (a second UV set, texture transforms, material
//! extensions), with example model names.
//!
//! usage: uvcheck <package folder> [model name substring]
use std::collections::BTreeMap;

use msfs2xp::bgl::modellib::ModelLibrary;

fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let only = std::env::args().nth(2).map(|s| s.to_ascii_lowercase());
    let mut tally: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    let mut note = |key: String, model: &str| {
        let e = tally.entry(key).or_default();
        e.0 += 1;
        if e.1.len() < 4 && !e.1.iter().any(|m| m == model) {
            e.1.push(model.to_string());
        }
    };
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) {
            continue;
        }
        let Ok(lib) = ModelLibrary::open(e.path()) else { continue };
        for g in lib.guids() {
            let Ok(info) = lib.info(g) else { continue };
            if only.as_ref().is_some_and(|o| !info.name.to_ascii_lowercase().contains(o.as_str())) {
                continue;
            }
            let Ok(glb) = lib.load_lod(g, 0) else { continue };
            if glb.len() < 20 {
                continue;
            }
            let len = u32::from_le_bytes(glb[12..16].try_into()?) as usize;
            let Some(text) = glb.get(20..20 + len) else { continue };
            // The JSON chunk is padded with spaces or NULs.
            let text = &text[..text.iter().rposition(|&c| c != 0 && c != b' ').map_or(0, |p| p + 1)];
            let Ok(j) = serde_json::from_slice::<serde_json::Value>(text) else { continue };
            let name = info.name.as_str();
            // With a model filter, also print every material in full.
            if only.is_some() {
                println!("MODEL {name}");
                for m in j["materials"].as_array().into_iter().flatten() {
                    let tex = m["pbrMetallicRoughness"]["baseColorTexture"]["index"]
                        .as_u64()
                        .and_then(|t| j["textures"][t as usize]["extensions"]["MSFT_texture_dds"]["source"].as_u64().or(j["textures"][t as usize]["source"].as_u64()))
                        .and_then(|i| j["images"][i as usize]["uri"].as_str())
                        .unwrap_or("-");
                    let ext = serde_json::to_string(&m["extensions"])?;
                    println!(
                        "  {:<28} base {:<44} alpha {:<6} two-sided {:<5} ext {}",
                        m["name"].as_str().unwrap_or("?"),
                        tex.rsplit(['\\', '/']).next().unwrap_or(tex),
                        m["alphaMode"].as_str().unwrap_or("OPAQUE"),
                        m["doubleSided"].as_bool().unwrap_or(false),
                        &ext[..ext.len().min(220)]
                    );
                }
            }
            for m in j["materials"].as_array().into_iter().flatten() {
                for (k, _) in m["extensions"].as_object().into_iter().flatten() {
                    note(format!("material extension {k}"), name);
                }
                for (k, _) in m["extras"].as_object().into_iter().flatten() {
                    note(format!("material extra {k}"), name);
                }
                let pbr = &m["pbrMetallicRoughness"];
                for (slot, t) in [
                    ("baseColor", &pbr["baseColorTexture"]),
                    ("metalRough", &pbr["metallicRoughnessTexture"]),
                    ("normal", &m["normalTexture"]),
                    ("emissive", &m["emissiveTexture"]),
                    ("occlusion", &m["occlusionTexture"]),
                ] {
                    if t.is_null() {
                        continue;
                    }
                    let tc = t["texCoord"].as_u64().unwrap_or(0);
                    if tc != 0 {
                        note(format!("{slot} texture uses TEXCOORD_{tc}"), name);
                    }
                    if let Some(x) = t["extensions"]["KHR_texture_transform"].as_object() {
                        note(format!("{slot} KHR_texture_transform {}", serde_json::to_string(x)?), name);
                    }
                }
                if let Some(uv) = m["extensions"]["ASOBO_material_UV_options"].as_object() {
                    note(format!("UV options {}", serde_json::to_string(uv)?), name);
                }
            }
            for mesh in j["meshes"].as_array().into_iter().flatten() {
                for p in mesh["primitives"].as_array().into_iter().flatten() {
                    let attrs: Vec<&str> = p["attributes"].as_object().into_iter().flatten().map(|(k, _)| k.as_str()).filter(|k| k.starts_with("TEXCOORD")).collect();
                    note(format!("primitive UV sets {attrs:?}"), name);
                }
            }
        }
    }
    let mut rows: Vec<_> = tally.into_iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1 .0));
    for (k, (n, models)) in rows.iter().take(60) {
        println!("{n:>7}  {k}\n           e.g. {}", models.join(", "));
    }
    Ok(())
}
