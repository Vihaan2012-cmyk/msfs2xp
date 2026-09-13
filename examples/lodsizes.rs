//! Development helper: each matching model's LOD screen-size thresholds,
//! triangles per LOD and bounding radius, for tuning X-Plane draw distances.
//!
//! usage: lodsizes <package folder> <name substring>...
use msfs2xp::bgl::modellib::ModelLibrary;
use msfs2xp::model3d::load_glb;

fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let wants: Vec<String> = std::env::args().skip(2).map(|s| s.to_ascii_lowercase()).collect();
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
            let mut cols = Vec::new();
            let mut radius = 0.0f32;
            for (i, lod) in info.lods.iter().enumerate() {
                let m = lib.load_lod(g, i).ok().and_then(|b| load_glb(&b).ok());
                let tris = m.as_ref().map_or(0, |m| m.triangle_count());
                if i == 0 {
                    if let Some(m) = &m {
                        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                        for v in m.meshes.iter().flat_map(|mesh| &mesh.vertices) {
                            for k in 0..3 {
                                lo[k] = lo[k].min(v.pos[k]);
                                hi[k] = hi[k].max(v.pos[k]);
                            }
                        }
                        radius = ((0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f32>()).sqrt() / 2.0;
                    }
                }
                cols.push(format!("{}:{}", lod.min_size, tris));
            }
            let extent = lib
                .load_lod(g, 0)
                .ok()
                .and_then(|b| load_glb(&b).ok())
                .and_then(|m| m.bounds())
                .map(|(lo, hi)| format!("x {:.1}..{:.1} y {:.1}..{:.1} z {:.1}..{:.1}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]))
                .unwrap_or_default();
            println!("{:<44} r={:6.1}  {}  {}", info.name, radius, cols.join("  "), extent);
        }
    }
    Ok(())
}
