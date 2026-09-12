//! Development helper: find known model IDs inside SimProp container files.
use std::collections::HashMap;
use msfs2xp::bgl::guid::Guid;
use msfs2xp::bgl::modellib::ModelLibrary;

fn f32s(b: &[u8]) -> String {
    b.chunks_exact(4).map(|c| format!("{:.3}", f32::from_le_bytes([c[0], c[1], c[2], c[3]]))).collect::<Vec<_>>().join(" ")
}

fn main() -> anyhow::Result<()> {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("package folder"));
    let only = std::env::args().nth(2).unwrap_or_default().to_ascii_lowercase();
    let mut names: HashMap<[u8; 16], String> = HashMap::new();
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) { continue; }
        if let Ok(lib) = ModelLibrary::open(e.path()) {
            for g in lib.guids() {
                if let Ok(i) = lib.info(g) { names.insert(g.0, i.name.clone()); }
            }
        }
    }
    let _ = Guid::from_slice;
    let mut files = 0; let mut hits_total = 0;
    for e in walkdir::WalkDir::new(root.join("SimPropContainers")).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("spb")) { continue; }
        let fname = e.file_name().to_string_lossy().to_string();
        let b = std::fs::read(e.path())?;
        files += 1;
        let mut hits = Vec::new();
        for at in 0..b.len().saturating_sub(16) {
            let k: [u8; 16] = b[at..at + 16].try_into()?;
            if let Some(n) = names.get(&k) { hits.push((at, n.clone())); }
        }
        hits_total += hits.len();
        if !only.is_empty() && !fname.to_ascii_lowercase().contains(&only) { continue; }
        println!("== {fname} ({} bytes): {} model refs", b.len(), hits.len());
        for (at, n) in hits.iter().take(12) {
            let before = &b[at.saturating_sub(24)..*at];
            let after = &b[at + 16..(at + 16 + 48).min(b.len())];
            println!("  @{at:5} {n:<40} before[{}] | after f32 [{}]", before.iter().map(|x| format!("{x:02x}")).collect::<String>(), f32s(after));
        }
    }
    println!("{files} container files, {hits_total} model references found");
    Ok(())
}
