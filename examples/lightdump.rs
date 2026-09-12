//! Development helper: how MSFS models describe lights. Prints nodes and
//! materials carrying light-related extensions or emissive settings.
use msfs2xp::bgl::modellib::ModelLibrary;

fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let wants: Vec<String> = std::env::args().skip(2).map(|s| s.to_ascii_lowercase()).collect();
    let mut shown = 0;
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) {
            continue;
        }
        let Ok(lib) = ModelLibrary::open(e.path()) else { continue };
        for g in lib.guids() {
            let Ok(info) = lib.info(g) else { continue };
            let n = info.name.to_ascii_lowercase();
            if !wants.iter().any(|w| n.contains(w.as_str())) {
                continue;
            }
            let Ok(glb) = lib.load_lod(g, 0) else { continue };
            let len = u32::from_le_bytes(glb[12..16].try_into()?) as usize;
            let text = String::from_utf8_lossy(&glb[20..20 + len]).trim_end_matches(['\0', ' ']).to_string();
            let j: serde_json::Value = serde_json::from_str(&text)?;
            println!("MODEL {}", info.name);
            for (i, node) in j["nodes"].as_array().into_iter().flatten().enumerate() {
                let ext = &node["extensions"];
                if ext.as_object().is_some_and(|o| o.keys().any(|k| k.to_ascii_lowercase().contains("light"))) {
                    println!("  node {i} {} translation {} rotation {} ext {}", node["name"], node["translation"], node["rotation"], ext);
                }
            }
            for (i, m) in j["materials"].as_array().into_iter().flatten().enumerate() {
                let ext = &m["extensions"];
                let lighty = ext.as_object().is_some_and(|o| o.keys().any(|k| {
                    let k = k.to_ascii_lowercase();
                    k.contains("light") || k.contains("emissive") || k.contains("day_night")
                }));
                if lighty || !m["emissiveTexture"].is_null() || !m["emissiveFactor"].is_null() {
                    println!(
                        "  mat {i} {} emissiveFactor {} emissiveTexture {} ext {}",
                        m["name"], m["emissiveFactor"], m["emissiveTexture"], ext
                    );
                }
            }
            shown += 1;
            if shown >= 6 {
                return Ok(());
            }
        }
    }
    Ok(())
}
