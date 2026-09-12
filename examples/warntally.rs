//! Development helper: tally loader warnings (skipped primitives) across models.
use std::collections::BTreeMap;
use msfs2xp::bgl::modellib::ModelLibrary;
use msfs2xp::model3d::load_glb;
fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut worst: Vec<(usize, String)> = Vec::new();
    let (mut models, mut failed) = (0, 0);
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) { continue; }
        let Ok(lib) = ModelLibrary::open(e.path()) else { continue };
        for g in lib.guids() {
            let Ok(info) = lib.info(g) else { continue };
            models += 1;
            match lib.load_lod(g, 0).map_err(|e| e.to_string()).and_then(|b| load_glb(&b).map_err(|e| e.to_string())) {
                Ok(m) => {
                    let skipped = m.warnings.iter().filter(|w| w.contains("skipped") && !w.contains("invisible")).count();
                    for w in &m.warnings {
                        let k = w.split(": ").last().unwrap_or(w).to_string();
                        *kinds.entry(k).or_default() += 1;
                    }
                    if skipped > 0 { worst.push((skipped, info.name.clone())); }
                }
                Err(_) => failed += 1,
            }
        }
    }
    println!("{models} models, {failed} failed to load");
    for (k, v) in &kinds { println!("  {v:6}  {k}"); }
    worst.sort_by(|a, b| b.0.cmp(&a.0));
    println!("models with skipped primitives: {}", worst.len());
    for (n, name) in worst.iter().take(15) { println!("  {n:4} skipped  {name}"); }
    Ok(())
}
