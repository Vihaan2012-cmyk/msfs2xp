//! Development helper: where a package places models it does not carry, to
//! tell what kind of object each one is (masts, road lights, fences, cars).
//!
//! usage: missingstats <package folder> [how many]
use std::collections::{HashMap, HashSet};

use msfs2xp::bgl::guid::Guid;
use msfs2xp::bgl::modellib::ModelLibrary;
use msfs2xp::geo::{inverse, LatLon};

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() -> anyhow::Result<()> {
    let input = std::env::args().nth(1).expect("package folder");
    let top: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(40);
    for source in msfs2xp::package::discover(&input)? {
        let loaded = msfs2xp::package::load(&source, None);
        let mut have: HashSet<Guid> = HashSet::new();
        for path in &loaded.model_libraries {
            if let Ok(lib) = ModelLibrary::open(path) {
                have.extend(lib.guids().iter().copied());
            }
        }
        let stands: Vec<LatLon> = loaded.airports.iter().flat_map(|a| a.parkings.iter().map(|p| p.pos)).collect();
        let runways: Vec<LatLon> = loaded
            .airports
            .iter()
            .flat_map(|a| {
                // Points every 100 m along each runway's centreline.
                a.runways.iter().flat_map(|r| {
                    let half = r.length_m as f64 / 2.0;
                    (0..=(r.length_m as usize / 100)).map(move |i| {
                        msfs2xp::geo::destination(r.centre, r.heading_true as f64, i as f64 * 100.0 - half)
                    })
                })
            })
            .collect();
        // Every placement, with whether its model is missing, for plotting.
        if let Ok(csv) = std::env::var("MISSING_CSV") {
            use std::fmt::Write as _;
            let mut out = String::from("guid,lat,lon,heading,alt,scale,missing\n");
            for p in &loaded.placements {
                let _ = writeln!(
                    out,
                    "{},{:.7},{:.7},{:.1},{:.1},{:.2},{}",
                    p.guid,
                    p.lat,
                    p.lon,
                    p.heading,
                    p.alt_m,
                    p.scale,
                    u8::from(!have.contains(&p.guid))
                );
            }
            std::fs::write(csv, out)?;
        }
        let mut by: HashMap<Guid, Vec<&msfs2xp::bgl::records::scenery::RawPlacement>> = HashMap::new();
        for p in loaded.placements.iter().filter(|p| !have.contains(&p.guid)) {
            by.entry(p.guid).or_default().push(p);
        }
        let mut rows: Vec<_> = by.into_iter().collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1.len()));
        println!("{}: {} missing models", source.name, rows.len());
        println!("{:>5} {:<40} {:>6} {:>6} {:>7} {:>7} {:>7} {:>6}", "n", "guid", "scale", "alt", "nn_m", "stand_m", "rwy_m", "agl");
        for (g, ps) in rows.iter().take(top) {
            let pos: Vec<LatLon> = ps.iter().map(|p| LatLon::new(p.lat, p.lon)).collect();
            // Nearest neighbour of the same model, on a sample for speed.
            let step = (pos.len() / 200).max(1);
            let nn = median(
                pos.iter()
                    .step_by(step)
                    .map(|a| {
                        pos.iter()
                            .filter(|b| *b != a)
                            .map(|b| inverse(*a, *b).0)
                            .fold(f64::INFINITY, f64::min)
                    })
                    .collect(),
            );
            let near = |set: &[LatLon]| {
                median(
                    pos.iter()
                        .step_by(step)
                        .map(|a| set.iter().map(|b| inverse(*a, *b).0).fold(f64::INFINITY, f64::min))
                        .collect(),
                )
            };
            println!(
                "{:>5} {:<40} {:>6.2} {:>6.1} {:>7.1} {:>7.0} {:>7.0} {:>6.2}",
                ps.len(),
                g.to_string(),
                median(ps.iter().map(|p| p.scale as f64).collect()),
                median(ps.iter().map(|p| p.alt_m).collect()),
                nn,
                near(&stands),
                near(&runways),
                ps.iter().filter(|p| p.agl).count() as f64 / ps.len() as f64
            );
        }
    }
    Ok(())
}
