//! Development helper: model-space bounding boxes of named library models.
use msfs2xp::bgl::modellib::ModelLibrary;
use msfs2xp::model3d::load_glb;
fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let wants: Vec<String> = std::env::args().skip(2).map(|s| s.to_ascii_lowercase()).collect();
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) { continue; }
        let Ok(lib) = ModelLibrary::open(e.path()) else { continue };
        for g in lib.guids() {
            let Ok(info) = lib.info(g) else { continue };
            let n = info.name.to_ascii_lowercase();
            if !wants.contains(&n) { continue; }
            let lod = info.lods.len().saturating_sub(1);
            let glb = lib.load_lod(g, lod)?;
            let m = load_glb(&glb)?;
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for mesh in &m.meshes { for v in &mesh.vertices { for k in 0..3 { lo[k] = lo[k].min(v.pos[k]); hi[k] = hi[k].max(v.pos[k]); } } }
            println!("{:<30} lod {lod}: x {:.1} .. {:.1}  y {:.1} .. {:.1}  z {:.1} .. {:.1}", info.name, lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]);
        }
    }
    Ok(())
}
