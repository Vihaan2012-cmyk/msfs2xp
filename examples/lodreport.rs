//! Development helper: triangles per LOD and the LOD the converter picks
//! under a triangle budget.
use msfs2xp::bgl::modellib::ModelLibrary;
use msfs2xp::model3d::load_glb;
fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let budget: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let mut rows = Vec::new();
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) { continue; }
        let Ok(lib) = ModelLibrary::open(e.path()) else { continue };
        for g in lib.guids() {
            let Ok(info) = lib.info(g) else { continue };
            let mut tris = Vec::new();
            for lod in 0..info.lods.len() {
                let t = lib.load_lod(g, lod).ok().and_then(|b| load_glb(&b).ok()).map(|m| m.triangle_count()).unwrap_or(0);
                tris.push(t);
            }
            let pick = tris.iter().position(|&t| t <= budget).unwrap_or(tris.len().saturating_sub(1));
            rows.push((pick, tris[0], info.name.clone(), tris));
        }
    }
    let total: usize = rows.iter().map(|r| r.3[r.0]).sum();
    let cliffs: Vec<_> = rows.iter().filter(|r| r.0 > 0 && r.3[r.0] * 20 < r.3[r.0 - 1]).map(|r| (r.2.clone(), r.3[r.0], r.3[r.0 - 1])).collect();
    println!("triangles across all models at their chosen LOD: {:.1}M", total as f64 / 1e6);
    println!("{} models land on a LOD with under 5% of the next finer one:", cliffs.len());
    for (n, t, prev) in cliffs.iter().take(15) { println!("  {n:<42} {t} (next finer: {prev})"); }
    let stepped = rows.iter().filter(|r| r.0 > 0).count();
    println!("{} models; {} stepped below LOD0 at a {} triangle budget", rows.len(), stepped, budget);
    let mut by_pick = std::collections::BTreeMap::new();
    for r in &rows { *by_pick.entry(r.0).or_insert(0) += 1; }
    println!("models by chosen LOD: {by_pick:?}");
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    for (pick, _, name, tris) in rows.iter().filter(|r| r.0 > 0).take(40) {
        let pct = tris[*pick] as f64 * 100.0 / tris[0].max(1) as f64;
        println!("  LOD{pick} ({pct:5.1}% of LOD0)  {name:<42} {:?}", tris);
    }
    Ok(())
}
