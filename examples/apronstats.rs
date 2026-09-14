//! Development helper: how an airport's aprons break down, for planning the
//! pavement merge and the decal choices. Drawn aprons by draw priority and
//! surface; decal aprons by material, with what the converter does with them
//! and their flag bytes.
//!
//! usage: apronstats <package folder> [material substring]
use std::collections::BTreeMap;

use msfs2xp::decals::{classify, is_square_quad, DecalKind};

fn main() -> anyhow::Result<()> {
    let input = std::env::args().nth(1).expect("package folder");
    let only = std::env::args().nth(2).map(|s| s.to_ascii_lowercase());
    for source in msfs2xp::package::discover(&input)? {
        let loaded = msfs2xp::package::load(&source, None);
        for ap in &loaded.airports {
            let mut drawn: BTreeMap<(u32, String), usize> = BTreeMap::new();
            let mut decals: BTreeMap<(String, &'static str), (usize, f64)> = BTreeMap::new();
            let mut flag_counts: BTreeMap<String, BTreeMap<(u8, bool), usize>> = BTreeMap::new();
            for a in &ap.aprons {
                if a.draw {
                    *drawn.entry((a.priority, format!("{:?}", a.surface))).or_default() += 1;
                    continue;
                }
                let Some(name) = a.material_name.clone() else { continue };
                let stretched = a.flags & 0x80 != 0 && is_square_quad(&a.vertices);
                let what = match classify(&name) {
                    None => "skipped (designator or atlas)",
                    Some(DecalKind::Grime) if !stretched => "skipped (tiled grime)",
                    Some(DecalKind::Grime) => "grime patch",
                    Some(DecalKind::Marking) => "marking",
                    Some(DecalKind::Ground) => "ground",
                };
                // Flag byte and whether the outline is a square quad, per material,
                // to look for a bit that flips the texture.
                *flag_counts
                    .entry(name.clone())
                    .or_default()
                    .entry((a.flags, is_square_quad(&a.vertices)))
                    .or_default() += 1;
                let e = decals.entry((name, what)).or_default();
                e.0 += 1;
                e.1 = a.uv_scale as f64;
            }
            println!("{}: {} aprons", ap.icao, ap.aprons.len());
            if only.is_none() {
                println!("  drawn aprons by (priority, surface), {} groups:", drawn.len());
                for ((p, s), n) in drawn.iter().filter(|(_, n)| **n >= 50) {
                    println!("    {p:>4} {s:<12} {n}");
                }
                let mut rows: Vec<_> = decals.into_iter().collect();
                rows.sort_by_key(|r| std::cmp::Reverse(r.1 .0));
                println!("  decal aprons by material:");
                for ((name, what), (n, scale)) in rows.iter().take(30) {
                    println!("    {n:>6} {name:<36} {what:<30} uv_scale {scale}");
                }
            }
            println!("  flags (value, square quad) by material:");
            for (name, flags) in &flag_counts {
                if only.as_ref().is_some_and(|o| !name.to_ascii_lowercase().contains(o.as_str())) {
                    continue;
                }
                let parts: Vec<String> = flags.iter().map(|((f, sq), n)| format!("0x{f:02X}{} x{n}", if *sq { " sq" } else { "" })).collect();
                println!("    {name:<36} {}", parts.join(", "));
            }
        }
    }
    Ok(())
}
