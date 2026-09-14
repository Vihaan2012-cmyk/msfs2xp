//! Development helper: an airport's drawn ground polygons by draw priority,
//! surface and material, with their area, to see what paints over what.
//!
//! usage: groundmats <package folder> [material substring]
use std::collections::BTreeMap;

fn area_m2(v: &[msfs2xp::geo::LatLon]) -> f64 {
    if v.len() < 3 {
        return 0.0;
    }
    let k = 111_320.0 * v[0].lat.to_radians().cos();
    let mut s = 0.0;
    for i in 0..v.len() {
        let (a, b) = (v[i], v[(i + 1) % v.len()]);
        s += a.lon * k * b.lat * 110_540.0 - b.lon * k * a.lat * 110_540.0;
    }
    s.abs() / 2.0
}

fn main() -> anyhow::Result<()> {
    let input = std::env::args().nth(1).expect("package folder");
    let only = std::env::args().nth(2).map(|s| s.to_ascii_lowercase());
    for source in msfs2xp::package::discover(&input)? {
        let loaded = msfs2xp::package::load(&source, None);
        for ap in &loaded.airports {
            let mut rows: BTreeMap<(u32, String, String, u8), (usize, f64)> = BTreeMap::new();
            for a in ap.aprons.iter().filter(|a| a.draw) {
                let name = a.material_name.clone().unwrap_or_else(|| "-".into());
                if only.as_ref().is_some_and(|o| !name.to_ascii_lowercase().contains(o.as_str())) {
                    continue;
                }
                let e = rows.entry((a.priority, format!("{:?}", a.surface), name, a.flags)).or_default();
                e.0 += 1;
                e.1 += area_m2(&a.vertices);
            }
            println!("{}: drawn ground polygons by priority, surface, material, flags", ap.icao);
            for ((prio, surface, name, flags), (n, area)) in rows {
                println!("  {prio:>11} {surface:<9} {name:<34} 0x{flags:02X} {n:>6} {:>9.3} km2", area / 1e6);
            }
        }
    }
    Ok(())
}
