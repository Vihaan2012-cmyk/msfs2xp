//! Development helper: which taxi paths ask to draw pavement, by kind,
//! material and surface, with their total area (length × width); and where
//! parking paths end, read as a shared point/stand index or as a stand index.
//!
//! usage: pathflags <package folder>
use std::collections::BTreeMap;

use msfs2xp::geo::LatLon;
use msfs2xp::model::{NodeKind, PathKind};

fn metres(a: LatLon, b: LatLon) -> f64 {
    let k = 111_320.0 * a.lat.to_radians().cos();
    (((a.lon - b.lon) * k).powi(2) + ((a.lat - b.lat) * 110_540.0).powi(2)).sqrt()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

fn main() -> anyhow::Result<()> {
    let input = std::env::args().nth(1).expect("package folder");
    for source in msfs2xp::package::discover(&input)? {
        let loaded = msfs2xp::package::load(&source, None);
        for ap in &loaded.airports {
            let mut rows: BTreeMap<(String, bool, String), (usize, f64)> = BTreeMap::new();
            for p in &ap.taxi_paths {
                let (Some(a), Some(b)) = (ap.taxi_nodes.get(p.start), ap.taxi_nodes.get(p.end)) else { continue };
                let material = p.material_name.clone().unwrap_or_else(|| {
                    p.material_guid.map_or("-".into(), |g| format!("unresolved {g}"))
                });
                let e = rows.entry((format!("{:?}", p.kind), p.draw_surface, material)).or_default();
                e.0 += 1;
                e.1 += metres(a.pos, b.pos) * p.width_m as f64;
            }
            let points = ap.taxi_nodes.len() - ap.parkings.len();
            println!("{}: {} taxi paths, {points} taxi points, {} stands", ap.icao, ap.taxi_paths.len(), ap.parkings.len());
            println!("  {:<14} {:>5} {:>6} {:>9}", "kind", "draws", "count", "area km2");
            for ((kind, draws, material), (n, area)) in rows {
                println!("  {kind:<14} {draws:>5} {n:>6} {:>9.3}  {material}", area / 1e6);
            }

            // Parking paths: does the end land on a stand, and how long are they
            // when the end is read either way?
            for draws in [true, false] {
                let mut on_stand = 0;
                let mut max_end = 0;
                let mut widths = Vec::new();
                let mut shared = Vec::new();
                let mut stand_index = Vec::new();
                let mut n = 0;
                for p in ap.taxi_paths.iter().filter(|p| p.kind == PathKind::Parking && p.draw_surface == draws) {
                    n += 1;
                    max_end = max_end.max(p.end);
                    widths.push(p.width_m as f64);
                    let Some(a) = ap.taxi_nodes.get(p.start) else { continue };
                    if let Some(b) = ap.taxi_nodes.get(p.end) {
                        if b.kind == NodeKind::Parking {
                            on_stand += 1;
                        }
                        shared.push(metres(a.pos, b.pos));
                    }
                    if let Some(s) = ap.parkings.get(p.end) {
                        stand_index.push(metres(a.pos, s.pos));
                    }
                }
                if n == 0 {
                    continue;
                }
                println!(
                    "  parking paths draws={draws}: {n}, end on a stand {on_stand}, largest end {max_end}, median width {:.1} m",
                    median(widths)
                );
                println!(
                    "    median length: as shared index {:.0} m, as stand index {:.0} m",
                    median(shared),
                    median(stand_index)
                );
            }
        }
    }
    Ok(())
}
