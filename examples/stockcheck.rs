//! Development helper: how many missing models an MSFS 2020 install's stock
//! model libraries could supply.
use std::collections::HashMap;
use msfs2xp::bgl::guid::Guid;
use msfs2xp::bgl::modellib::ModelLibrary;
use msfs2xp::model3d::load_glb;

fn main() -> anyhow::Result<()> {
    let stock = std::env::args().nth(1).expect("OneStore folder");
    let reports: Vec<String> = std::env::args().skip(2).collect();
    let mut libs = Vec::new();
    for e in walkdir::WalkDir::new(&stock).max_depth(5).into_iter().filter_map(Result::ok) {
        let p = e.path();
        if !p.to_string_lossy().to_ascii_lowercase().contains("modellib") { continue; }
        if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) { continue; }
        match ModelLibrary::open(p) {
            Ok(l) => { println!("stock library {} ({} models)", p.display(), l.guids().len()); libs.push(l) }
            Err(err) => println!("unreadable {}: {err}", p.display()),
        }
    }
    let mut index: HashMap<Guid, usize> = HashMap::new();
    for (i, l) in libs.iter().enumerate() { for g in l.guids() { index.insert(*g, i); } }
    for r in reports {
        let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&r)?)?;
        let missing = j["missing_models"].as_array().cloned().unwrap_or_default();
        let (mut found, mut placements, mut total, mut loads) = (0, 0, 0, 0);
        let mut examples = Vec::new();
        for m in &missing {
            let g: Guid = m[0].as_str().unwrap_or_default().parse().map_err(anyhow::Error::msg)?;
            let n = m[1].as_u64().unwrap_or(0) as usize;
            total += n;
            if let Some(&i) = index.get(&g) {
                found += 1; placements += n;
                let name = libs[i].info(&g).map(|x| x.name.clone()).unwrap_or_default();
                let ok = libs[i].load_lod(&g, 0).ok().and_then(|b| load_glb(&b).ok()).map(|m| m.triangle_count());
                if ok.is_some() { loads += 1; }
                if examples.len() < 12 { examples.push(format!("{n:5} x {name} ({})", ok.map(|t| format!("{t} tris")).unwrap_or("does not load".into()))); }
            }
        }
        println!("== {r}\n   {found} of {} missing models are in the stock libraries ({loads} load), covering {placements} of {total} placements", missing.len());
        for e in examples { println!("   {e}"); }
    }
    Ok(())
}
