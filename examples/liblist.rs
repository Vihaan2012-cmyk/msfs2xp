//! Development helper: list library models whose names match, and whether an
//! X-Plane object with the same name was written.
use msfs2xp::bgl::modellib::ModelLibrary;
fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).expect("package folder");
    let objects = std::path::PathBuf::from(std::env::args().nth(2).expect("objects folder"));
    let pat = regex_lite(std::env::args().nth(3).unwrap_or_default());
    let written: Vec<String> = std::fs::read_dir(&objects)?.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_ascii_lowercase()).collect();
    let (mut total, mut hit, mut conv) = (0, 0, 0);
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) { continue; }
        let Ok(lib) = ModelLibrary::open(e.path()) else { continue };
        for g in lib.guids() {
            total += 1;
            let Ok(info) = lib.info(g) else { continue };
            let n = info.name.to_ascii_lowercase();
            if !pat.iter().any(|p| n.contains(p.as_str())) { continue; }
            hit += 1;
            let stem: String = info.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect::<String>().to_ascii_lowercase();
            let done = written.iter().any(|w| w.starts_with(&format!("{stem}_")) && w.ends_with(".obj"));
            conv += done as usize;
            println!("{} {:<45} {}", if done { "CONVERTED  " } else { "NOT PLACED " }, info.name, g);
        }
    }
    println!("{total} models in libraries; {hit} match; {conv} converted");
    Ok(())
}
fn regex_lite(s: String) -> Vec<String> { s.to_ascii_lowercase().split('|').filter(|x| !x.is_empty()).map(String::from).collect() }
