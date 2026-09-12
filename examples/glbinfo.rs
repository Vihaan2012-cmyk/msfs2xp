//! Development helper: summarise glTF features used by a package's models.
use std::collections::BTreeMap;
use msfs2xp::bgl::modellib::ModelLibrary;

fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let want = std::env::args().nth(2).unwrap_or_default().to_ascii_lowercase();
    let mut ext: BTreeMap<String, usize> = BTreeMap::new();
    let mut pos: BTreeMap<String, usize> = BTreeMap::new();
    let mut models = 0usize;
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) { continue; }
        let Ok(lib) = ModelLibrary::open(e.path()) else { continue };
        for g in lib.guids() {
            let Ok(info) = lib.info(&g) else { continue };
            let Ok(glb) = lib.load_lod(&g, 0) else { continue };
            models += 1;
            let len = u32::from_le_bytes(glb[12..16].try_into()?) as usize;
            let text = String::from_utf8_lossy(&glb[20..20 + len]).trim_end_matches(['\0', ' ']).to_string();
            let Ok(j) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            for x in j["extensionsUsed"].as_array().into_iter().flatten() {
                *ext.entry(x.as_str().unwrap_or("?").to_string()).or_default() += 1;
            }
            let mut mine: BTreeMap<String, usize> = BTreeMap::new();
            for m in j["meshes"].as_array().into_iter().flatten() {
                for p in m["primitives"].as_array().into_iter().flatten() {
                    let a = &j["accessors"][p["attributes"]["POSITION"].as_u64().unwrap_or(0) as usize];
                    let bv = &j["bufferViews"][a["bufferView"].as_u64().unwrap_or(0) as usize];
                    let k = format!("ctype={} norm={} type={} stride={} sparse={} bvext={}",
                        a["componentType"], a["normalized"], a["type"], bv["byteStride"], !a["sparse"].is_null(),
                        bv["extensions"].as_object().map(|o| o.keys().cloned().collect::<Vec<_>>().join("+")).unwrap_or_default());
                    *pos.entry(k.clone()).or_default() += 1;
                    *mine.entry(k).or_default() += 1;
                }
            }
            if !want.is_empty() && info.name.to_ascii_lowercase().contains(&want) {
                println!("MODEL {} ({} LODs) in {}", info.name, info.lods.len(), e.path().display());
                println!("  extensionsUsed {}", j["extensionsUsed"]);
                println!("  position formats {mine:?}");
                println!("  first node {}", j["nodes"][0]);
                let a = &j["accessors"][j["meshes"][0]["primitives"][0]["attributes"]["POSITION"].as_u64().unwrap_or(0) as usize];
                println!("  first POSITION accessor {a}");
            }
        }
    }
    println!("{models} models; extensionsUsed {ext:?}");
    println!("position formats {pos:#?}");
    Ok(())
}
