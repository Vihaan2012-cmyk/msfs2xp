//! Development helper: dump the glTF primitive and accessor JSON of a model.
use msfs2xp::bgl::modellib::ModelLibrary;
fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let want = std::env::args().nth(2).expect("model name").to_ascii_lowercase();
    let lod: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) { continue; }
        let Ok(lib) = ModelLibrary::open(e.path()) else { continue };
        for g in lib.guids() {
            let Ok(info) = lib.info(g) else { continue };
            if info.name.to_ascii_lowercase() != want { continue; }
            let glb = lib.load_lod(g, lod)?;
            let len = u32::from_le_bytes(glb[12..16].try_into()?) as usize;
            let text = String::from_utf8_lossy(&glb[20..20 + len]).trim_end_matches(['\0', ' ']).to_string();
            let j: serde_json::Value = serde_json::from_str(&text)?;
            let meshes = j["meshes"].as_array().map(|a| a.len()).unwrap_or(0);
            let prims: usize = j["meshes"].as_array().into_iter().flatten().map(|m| m["primitives"].as_array().map(|a| a.len()).unwrap_or(0)).sum();
            println!("{} LOD{lod}: {meshes} meshes, {prims} primitives, {} accessors, {} bufferViews", info.name, j["accessors"].as_array().map(|a| a.len()).unwrap_or(0), j["bufferViews"].as_array().map(|a| a.len()).unwrap_or(0));
            for (mi, m) in j["meshes"].as_array().into_iter().flatten().enumerate().take(2) {
                for (pi, p) in m["primitives"].as_array().into_iter().flatten().enumerate().take(3) {
                    println!("mesh {mi} prim {pi}: {}", p);
                    for key in ["POSITION"] {
                        if let Some(a) = p["attributes"][key].as_u64() { println!("   {key} accessor {a}: {}", j["accessors"][a as usize]); }
                    }
                    if let Some(a) = p["indices"].as_u64() {
                        let acc = &j["accessors"][a as usize];
                        println!("   indices accessor {a}: {}", acc);
                        println!("   indices bufferView: {}", j["bufferViews"][acc["bufferView"].as_u64().unwrap_or(0) as usize]);
                    }
                }
            }
            return Ok(());
        }
    }
    Ok(())
}
