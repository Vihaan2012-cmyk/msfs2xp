//! Development helper: the node tree of a package's models — transforms,
//! meshes, skins and animations — and optionally their glTF JSON.
//!
//! usage: nodetree <package folder> <model name substring> [dir to save JSON]
use msfs2xp::bgl::modellib::ModelLibrary;

fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let only = std::env::args().nth(2).expect("model name substring").to_ascii_lowercase();
    let save = std::env::args().nth(3);
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) {
            continue;
        }
        let Ok(lib) = ModelLibrary::open(e.path()) else { continue };
        for g in lib.guids() {
            let Ok(info) = lib.info(g) else { continue };
            if !info.name.to_ascii_lowercase().contains(&only) {
                continue;
            }
            let Ok(glb) = lib.load_lod(g, 0) else { continue };
            if glb.len() < 20 {
                continue;
            }
            let len = u32::from_le_bytes(glb[12..16].try_into()?) as usize;
            let Some(text) = glb.get(20..20 + len) else { continue };
            let text = &text[..text.iter().rposition(|&c| c != 0 && c != b' ').map_or(0, |p| p + 1)];
            let j: serde_json::Value = serde_json::from_slice(text)?;
            let count = |k: &str| j[k].as_array().map_or(0, Vec::len);
            println!(
                "MODEL {}  nodes {}  meshes {}  skins {}  animations {}  scenes {}",
                info.name,
                count("nodes"),
                count("meshes"),
                count("skins"),
                count("animations"),
                count("scenes")
            );
            if let Some(dir) = &save {
                std::fs::create_dir_all(dir)?;
                std::fs::write(std::path::Path::new(dir).join(format!("{}.json", info.name)), text)?;
            }
            let nodes = j["nodes"].as_array().cloned().unwrap_or_default();
            let roots: Vec<usize> = j["scenes"][0]["nodes"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_u64()).map(|v| v as usize).collect())
                .unwrap_or_default();
            fn show(nodes: &[serde_json::Value], i: usize, depth: usize, printed: &mut usize) {
                if *printed > 60 {
                    return;
                }
                *printed += 1;
                let n = &nodes[i];
                let fmt = |k: &str| {
                    n[k].as_array().map(|a| {
                        let v: Vec<String> = a.iter().map(|x| format!("{:.2}", x.as_f64().unwrap_or(0.0))).collect();
                        format!(" {k}[{}]", v.join(","))
                    })
                };
                println!(
                    "{}#{i} {:?}{}{}{}{}{}{}",
                    "  ".repeat(depth + 1),
                    n["name"].as_str().unwrap_or(""),
                    n["mesh"].as_u64().map_or(String::new(), |m| format!(" mesh={m}")),
                    n["skin"].as_u64().map_or(String::new(), |s| format!(" SKIN={s}")),
                    fmt("translation").unwrap_or_default(),
                    fmt("rotation").unwrap_or_default(),
                    fmt("scale").unwrap_or_default(),
                    if n.get("matrix").is_some() { " MATRIX" } else { "" },
                );
                for c in n["children"].as_array().into_iter().flatten().filter_map(|v| v.as_u64()) {
                    show(nodes, c as usize, depth + 1, printed);
                }
            }
            let mut printed = 0;
            for r in roots {
                show(&nodes, r, 0, &mut printed);
            }
            return Ok(());
        }
    }
    Ok(())
}
