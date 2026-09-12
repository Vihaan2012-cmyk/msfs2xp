//! Development helper: resolve the model IDs in 0x1B scenery records.
use std::collections::{BTreeMap, HashMap};
use msfs2xp::bgl::guid::Guid;
use msfs2xp::bgl::modellib::ModelLibrary;

fn u16_at(b: &[u8], at: usize) -> usize { u16::from_le_bytes([b[at], b[at + 1]]) as usize }
fn u32_at(b: &[u8], at: usize) -> usize { u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize }

fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let mut names: HashMap<Guid, String> = HashMap::new();
    let mut scenes = Vec::new();
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) { continue; }
        if let Ok(lib) = ModelLibrary::open(e.path()) {
            for g in lib.guids() { if let Ok(i) = lib.info(g) { names.insert(*g, i.name.clone()); } }
        }
        if std::fs::metadata(e.path())?.len() < 50_000_000 { scenes.push(e.path().to_path_buf()); }
    }
    println!("{} models named", names.len());
    let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
    let mut shown = 0;
    for f in scenes {
        let b = std::fs::read(&f)?;
        if b.len() < 0x38 { continue; }
        let n = u32_at(&b, 0x14);
        for i in 0..n {
            let s = 0x38 + 20 * i;
            if s + 20 > b.len() { break; }
            let (t, sf, cnt, off) = (u32_at(&b, s), u32_at(&b, s + 4), u32_at(&b, s + 8), u32_at(&b, s + 12));
            if t != 0x25 { continue; }
            let esz = if sf & 0x10000 != 0 { 20 } else { 16 };
            for j in 0..cnt {
                let e = off + esz * j;
                let (doff, dsize) = (u32_at(&b, e + esz - 8), u32_at(&b, e + esz - 4));
                let mut at = doff;
                while at + 4 <= doff + dsize {
                    let (id, rs) = (u16_at(&b, at), u16_at(&b, at + 2));
                    if rs < 4 { break; }
                    if id == 0x1B && rs >= 44 {
                        let rec = &b[at..at + rs];
                        let g = if rs >= 64 { Guid::from_slice(&rec[48..64]) } else { None };
                        let name = g.and_then(|g| names.get(&g).cloned()).unwrap_or_else(|| format!("NOT IN LIBRARY {}", g.map(|g| g.to_string()).unwrap_or_default()));
                        if shown < 6 {
                            println!("{} 0x1B: {} | bytes 44..64 {:02x?}", f.file_name().unwrap().to_string_lossy(), name, &rec[44..rs.min(64)]);
                            shown += 1;
                        }
                        *by_name.entry(name).or_default() += 1;
                    }
                    at += rs;
                }
            }
        }
    }
    let found: usize = by_name.iter().filter(|(k, _)| !k.starts_with("NOT IN")).map(|(_, v)| v).sum();
    let missing: usize = by_name.iter().filter(|(k, _)| k.starts_with("NOT IN")).map(|(_, v)| v).sum();
    println!("0x1B records: {found} resolve to library models, {missing} do not");
    for (k, v) in by_name.iter().filter(|(k, _)| !k.starts_with("NOT IN")).take(60) { println!("  {v:4} {k}"); }
    Ok(())
}
