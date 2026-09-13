//! Development helper: every jetway a package places, what model it names
//! and whether its parked shape could be read. For library models that could
//! not be measured, shows what the model library holds for them.
//!
//! usage: jetwaycheck <package folder>
use msfs2xp::bgl::modellib::{ModelCatalog, ModelLibrary};
use msfs2xp::model::JetwayModel;

fn main() -> anyhow::Result<()> {
    let input = std::env::args().nth(1).expect("package folder");
    for source in msfs2xp::package::discover(&input)? {
        let loaded = msfs2xp::package::load(&source, None);
        let mut catalog = ModelCatalog::default();
        for path in &loaded.model_libraries {
            if let Ok(lib) = ModelLibrary::open(path) {
                catalog.add(lib);
            }
        }
        for ap in &loaded.airports {
            let mut unmeasured = std::collections::BTreeMap::new();
            let mut total = 0;
            for p in &ap.parkings {
                for j in &p.jetways {
                    total += 1;
                    if j.spec.is_some() {
                        continue;
                    }
                    let what = match &j.model {
                        JetwayModel::SimObject(t) => format!("SimObject {t}"),
                        JetwayModel::Library(g) => match catalog.find(g) {
                            None => format!("library {g}: not in the package's model libraries"),
                            Some(lib) => {
                                let name = lib.info(g).map(|i| i.name).unwrap_or_default();
                                let xml = lib.xml(g).ok().flatten().unwrap_or_default();
                                format!(
                                    "library {g}: name {name:?}, XML {} bytes, IK chain {}",
                                    xml.len(),
                                    xml.contains("IK_MainHandle")
                                )
                            }
                        },
                        JetwayModel::Unknown => "unknown".to_string(),
                    };
                    *unmeasured.entry(what).or_insert(0) += 1;
                }
            }
            println!("{}: {total} jetways, {} unmeasured", ap.icao, unmeasured.values().sum::<usize>());
            for (what, n) in &unmeasured {
                println!("  {n:4}  {what}");
            }
        }
    }
    Ok(())
}
