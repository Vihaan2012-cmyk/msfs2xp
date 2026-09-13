//! Development helper: painted lines by MSFS style and material, to see which
//! the converter drops and what they are.
//!
//! usage: linestats <package folder>
use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    let input = std::env::args().nth(1).expect("package folder");
    for source in msfs2xp::package::discover(&input)? {
        let loaded = msfs2xp::package::load(&source, None);
        for ap in &loaded.airports {
            let mut rows: BTreeMap<(String, String), (usize, f64)> = BTreeMap::new();
            for l in &ap.painted_lines {
                let len: f64 = l.vertices.windows(2).map(|w| msfs2xp::geo::inverse(w[0], w[1]).0).sum();
                let e = rows
                    .entry((format!("{:?}", l.kind), l.material_name.clone().unwrap_or_else(|| "-".into())))
                    .or_default();
                e.0 += 1;
                e.1 += len;
            }
            // How far black outlines sit from the nearest other line: for each
            // black line, the distance from each vertex to the nearest vertex
            // or segment of a non-black line (sampled; metres).
            let plane = msfs2xp::geo::Plane::new(ap.datum);
            let is_black = |l: &msfs2xp::model::PaintedLine| {
                l.material_name.as_deref().is_some_and(|n| n.to_ascii_lowercase().contains("black"))
            };
            let others: Vec<((f64, f64), (f64, f64))> = ap
                .painted_lines
                .iter()
                .filter(|l| !is_black(l))
                .flat_map(|l| {
                    let xy: Vec<(f64, f64)> = l.vertices.iter().map(|v| plane.to_xy(*v)).collect();
                    xy.windows(2).map(|w| (w[0], w[1])).collect::<Vec<_>>()
                })
                .collect();
            let seg_dist = |p: (f64, f64), (a, b): ((f64, f64), (f64, f64))| {
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let l2 = dx * dx + dy * dy;
                let t = if l2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
            };
            let mut dists: BTreeMap<String, Vec<f64>> = BTreeMap::new();
            for l in ap.painted_lines.iter().filter(|l| is_black(l)).step_by(3) {
                for v in l.vertices.iter().step_by(4) {
                    let p = plane.to_xy(*v);
                    let d = others.iter().map(|&s| seg_dist(p, s)).fold(f64::INFINITY, f64::min);
                    dists.entry(l.material_name.clone().unwrap_or_default()).or_default().push(d);
                }
            }
            for (name, mut d) in dists {
                d.sort_by(f64::total_cmp);
                let q = |f: f64| d[((d.len() - 1) as f64 * f) as usize];
                println!(
                    "  black outline {name}: {} samples, distance to nearest other line p10 {:.2} median {:.2} p90 {:.2} m",
                    d.len(),
                    q(0.1),
                    q(0.5),
                    q(0.9)
                );
            }
            println!("{}: {} painted lines", ap.icao, ap.painted_lines.len());
            let mut v: Vec<_> = rows.into_iter().collect();
            v.sort_by_key(|r| std::cmp::Reverse(r.1 .0));
            for ((kind, mat), (n, len)) in v {
                println!("  {n:>5}  {len:>8.0} m  {kind:<18} {mat}");
            }
        }
    }
    Ok(())
}
