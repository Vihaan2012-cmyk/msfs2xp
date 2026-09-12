//! Development helper: print materials of models whose name matches.
use msfs2xp::bgl::modellib::ModelLibrary;
fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let wants: Vec<String> = std::env::args().skip(2).map(|s| s.to_ascii_lowercase()).collect();
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) { continue; }
        let Ok(lib) = ModelLibrary::open(e.path()) else { continue };
        for g in lib.guids() {
            let Ok(info) = lib.info(g) else { continue };
            let n = info.name.to_ascii_lowercase();
            if !wants.iter().any(|w| n.contains(w)) { continue; }
            let Ok(glb) = lib.load_lod(g, 0) else { continue };
            let len = u32::from_le_bytes(glb[12..16].try_into()?) as usize;
            let text = String::from_utf8_lossy(&glb[20..20 + len]).trim_end_matches(['\0', ' ']).to_string();
            let j: serde_json::Value = serde_json::from_str(&text)?;
            println!("MODEL {} ({} LODs)", info.name, info.lods.len());
            for (i, m) in j["materials"].as_array().into_iter().flatten().enumerate() {
                let tex = m["pbrMetallicRoughness"]["baseColorTexture"]["index"].as_u64()
                    .and_then(|t| j["textures"][t as usize]["extensions"]["MSFT_texture_dds"]["source"].as_u64().or(j["textures"][t as usize]["source"].as_u64()))
                    .and_then(|s| j["images"][s as usize]["uri"].as_str().map(|u| u.rsplit(|c: char| c == '/' || c as u32 == 92).next().unwrap_or(u).to_string()))
                    .unwrap_or_default();
                let exts: Vec<String> = m["extensions"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
                println!("  mat {i} {} alphaMode={} cutoff={} doubleSided={} tex={} exts={:?}",
                    m["name"], m["alphaMode"], m["alphaCutoff"], m["doubleSided"], tex, exts);
            }
        }
    }
    Ok(())
}
