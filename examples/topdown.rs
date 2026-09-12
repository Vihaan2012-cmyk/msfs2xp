//! Development helper: render library models from above, shaded by normal,
//! at several LODs, to compare detail levels.
use msfs2xp::bgl::modellib::ModelLibrary;
use msfs2xp::model3d::load_glb;

fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let out = std::path::PathBuf::from(std::env::args().nth(2).expect("output folder"));
    let jobs: Vec<(String, usize)> = std::env::args().skip(3).filter_map(|a| {
        let (n, l) = a.rsplit_once('@')?;
        Some((n.to_ascii_lowercase(), l.parse().ok()?))
    }).collect();
    std::fs::create_dir_all(&out)?;
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) { continue; }
        let Ok(lib) = ModelLibrary::open(e.path()) else { continue };
        for g in lib.guids() {
            let Ok(info) = lib.info(g) else { continue };
            for (name, lod) in jobs.iter().filter(|(n, _)| info.name.eq_ignore_ascii_case(n)) {
                let lod = (*lod).min(info.lods.len() - 1);
                let m = load_glb(&lib.load_lod(g, lod)?)?;
                let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
                for mesh in &m.meshes { for v in &mesh.vertices { lo[0] = lo[0].min(v.pos[0]); hi[0] = hi[0].max(v.pos[0]); lo[1] = lo[1].min(v.pos[2]); hi[1] = hi[1].max(v.pos[2]); } }
                let size = 900u32;
                let s = (size as f32 - 20.0) / (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-3);
                let mut img = image::RgbImage::from_pixel(size, size, image::Rgb([30, 60, 30]));
                let mut depth = vec![f32::MIN; (size * size) as usize];
                let light = [0.4f32, 0.8, 0.45];
                for mesh in &m.meshes {
                    for t in mesh.indices.chunks_exact(3) {
                        let p: Vec<[f32; 3]> = t.iter().map(|&i| mesh.vertices[i as usize].pos).collect();
                        let (ux, uy, uz) = (p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]);
                        let (vx, vy, vz) = (p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]);
                        let n = [uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx];
                        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
                        let shade = ((n[0] * light[0] + n[1] * light[1] + n[2] * light[2]) / len).abs();
                        let c = (40.0 + 200.0 * shade) as u8;
                        let sx: Vec<f32> = p.iter().map(|q| 10.0 + (q[0] - lo[0]) * s).collect();
                        let sy: Vec<f32> = p.iter().map(|q| 10.0 + (q[2] - lo[1]) * s).collect();
                        let (x0, x1) = (sx.iter().cloned().fold(f32::MAX, f32::min).max(0.0) as i32, sx.iter().cloned().fold(f32::MIN, f32::max).min(size as f32 - 1.0) as i32);
                        let (y0, y1) = (sy.iter().cloned().fold(f32::MAX, f32::min).max(0.0) as i32, sy.iter().cloned().fold(f32::MIN, f32::max).min(size as f32 - 1.0) as i32);
                        let area = (sx[1] - sx[0]) * (sy[2] - sy[0]) - (sx[2] - sx[0]) * (sy[1] - sy[0]);
                        if area.abs() < 1e-6 { continue; }
                        for y in y0..=y1 { for x in x0..=x1 {
                            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                            let w0 = ((sx[1] - fx) * (sy[2] - fy) - (sx[2] - fx) * (sy[1] - fy)) / area;
                            let w1 = ((sx[2] - fx) * (sy[0] - fy) - (sx[0] - fx) * (sy[2] - fy)) / area;
                            let w2 = 1.0 - w0 - w1;
                            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 { continue; }
                            let h = w0 * p[0][1] + w1 * p[1][1] + w2 * p[2][1];
                            let k = (y as u32 * size + x as u32) as usize;
                            if h > depth[k] { depth[k] = h; img.put_pixel(x as u32, y as u32, image::Rgb([c, c, c])); }
                        } }
                    }
                }
                let f = out.join(format!("{}_lod{lod}_{}k.png", info.name, m.triangle_count() / 1000));
                img.save(&f)?;
                println!("{}", f.display());
            }
        }
    }
    Ok(())
}
