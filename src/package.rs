//! Finding MSFS scenery packages and loading what they contain.
//!
//! A package can be several gigabytes, most of it textures and one or two huge
//! model libraries. Only the section table of each BGL is read up front, so the
//! loader can tell an airport file from a model library without pulling a
//! gigabyte into memory; only files with airport or placement sections are read
//! whole.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use walkdir::WalkDir;

use crate::bgl::file::{SECTION_AIRPORT, SECTION_AIRPORT_ALT, SECTION_SCENERY_OBJECT};
use crate::bgl::modellib::SECTION_MODEL_DATA;
use crate::bgl::guid::Guid;
use crate::bgl::records::scenery::{self, RawContainerPlacement, RawPlacement};
use crate::bgl::spb::{self, ContainerChild};
use crate::bgl::records::{parse_airport, Variant};
use crate::bgl::{classify, BglFile, FileClass};
use crate::geo::{inverse, LatLon, Plane};
use crate::materials::{self, MaterialCatalog};
use crate::model::{self, Airport, SimKind, Windsock};

/// One input unit: a package, or a loose set of BGL files.
#[derive(Debug, Clone)]
pub struct Source {
    /// Folder or file name, used for the output pack name.
    pub name: String,
    pub root: PathBuf,
    /// The manifest title, when there is one.
    pub title: Option<String>,
    pub files: Vec<PathBuf>,
    pub sim: SimKind,
}

impl Source {
    /// A human-friendly name for the output pack.
    pub fn display_name(&self) -> String {
        self.title.clone().unwrap_or_else(|| self.name.clone())
    }
}

/// Everything read out of one source.
#[derive(Debug)]
pub struct Loaded {
    pub source: Source,
    pub airports: Vec<Airport>,
    pub placements: Vec<RawPlacement>,
    pub model_libraries: Vec<PathBuf>,
    pub problems: Vec<String>,
    /// Informational messages about how the source was interpreted.
    pub notes: Vec<String>,
}

fn is_bgl(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("bgl"))
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

fn manifest_title(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("title")?
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn package(dir: &Path) -> Source {
    let mut files = Vec::new();
    let mut sim = SimKind::Msfs2020;
    for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
        let p = entry.path();
        if !entry.file_type().is_file() {
            continue;
        }
        // KTX2 textures and SimProp containers only exist in MSFS 2024 packages.
        if has_ext(p, "ktx2") || has_ext(p, "spb") {
            sim = SimKind::Msfs2024;
        }
        let in_temp = p.components().any(|c| c.as_os_str().eq_ignore_ascii_case("_temp"));
        if is_bgl(p) && !in_temp {
            files.push(p.to_path_buf());
        }
    }
    files.sort();
    Source {
        name: dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        root: dir.to_path_buf(),
        title: manifest_title(dir),
        files,
        sim,
    }
}

/// Where an installed simulator keeps its packages, from its UserCfg.opt.
fn installed_community(which: &str) -> anyhow::Result<PathBuf> {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let roaming = std::env::var("APPDATA").unwrap_or_default();
    let candidates: Vec<PathBuf> = match which {
        "2020" => vec![
            Path::new(&local).join("Packages/Microsoft.FlightSimulator_8wekyb3d8bbwe/LocalCache/UserCfg.opt"),
            Path::new(&roaming).join("Microsoft Flight Simulator/UserCfg.opt"),
        ],
        "2024" => vec![
            Path::new(&local).join("Packages/Microsoft.Limitless_8wekyb3d8bbwe/LocalCache/UserCfg.opt"),
            Path::new(&roaming).join("Microsoft Flight Simulator 2024/UserCfg.opt"),
        ],
        other => bail!("unknown simulator {other:?}; use auto:2020 or auto:2024"),
    };
    for cfg in candidates {
        let Ok(text) = std::fs::read_to_string(&cfg) else {
            continue;
        };
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("InstalledPackagesPath") {
                let path = rest.trim().trim_matches('"');
                return Ok(Path::new(path).join("Community"));
            }
        }
    }
    bail!("could not find an installed MSFS {which}")
}

/// Expand one command-line input into sources.
pub fn discover(input: &str) -> anyhow::Result<Vec<Source>> {
    if let Some(which) = input.strip_prefix("auto:") {
        let community = installed_community(which)?;
        return discover(&community.to_string_lossy());
    }
    let path = Path::new(input);
    if path.is_file() {
        if is_bgl(path) {
            return Ok(vec![Source {
                name: path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                root: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
                title: None,
                files: vec![path.to_path_buf()],
                sim: SimKind::Unknown,
            }]);
        }
        bail!(
            "{} is not a BGL file (SDK XML input is not supported yet)",
            path.display()
        );
    }
    if !path.is_dir() {
        bail!("{} does not exist", path.display());
    }
    if path.join("manifest.json").is_file() {
        return Ok(vec![package(path)]);
    }
    // A Community or OneStore folder: every child with a manifest is a package.
    let mut children: Vec<PathBuf> = std::fs::read_dir(path)
        .with_context(|| format!("reading {}", path.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("manifest.json").is_file())
        .collect();
    children.sort();
    if !children.is_empty() {
        return Ok(children.iter().map(|c| package(c)).collect());
    }
    // Otherwise treat the folder as a loose collection of BGL files.
    let mut src = package(path);
    src.sim = SimKind::Unknown;
    if src.files.is_empty() {
        bail!("no BGL files under {}", path.display());
    }
    Ok(vec![src])
}

/// The section types in a BGL, read from its header without loading the file.
fn section_kinds(path: &Path) -> Result<Vec<u32>, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut head = vec![0u8; 0x38];
    f.read_exact(&mut head).map_err(|_| "file too short".to_string())?;
    if classify(&head) != FileClass::Bgl {
        let mut sample = vec![0u8; 4096];
        let n = std::fs::File::open(path)
            .and_then(|mut g| g.read(&mut sample))
            .unwrap_or(0);
        return Err(match classify(&sample[..n]) {
            FileClass::Encrypted => "encrypted (marketplace DRM) - cannot be converted".into(),
            _ => "not a BGL file".into(),
        });
    }
    let count = u32::from_le_bytes([head[0x14], head[0x15], head[0x16], head[0x17]]) as usize;
    if count > 4096 {
        return Err(format!("implausible section count {count}"));
    }
    let mut table = vec![0u8; count * 20];
    f.read_exact(&mut table)
        .map_err(|_| "truncated section table".to_string())?;
    Ok(table
        .chunks_exact(20)
        .map(|t| u32::from_le_bytes([t[0], t[1], t[2], t[3]]))
        .collect())
}

/// The simulator's stock material libraries, read once per run.
fn stock_materials() -> &'static MaterialCatalog {
    static STOCK: std::sync::OnceLock<MaterialCatalog> = std::sync::OnceLock::new();
    STOCK.get_or_init(|| {
        let mut c = MaterialCatalog::default();
        c.load_stock();
        c
    })
}

/// Read every airport and placement in a source.
pub fn load(source: &Source, hint: Option<Variant>) -> Loaded {
    let mut loaded = Loaded {
        source: source.clone(),
        airports: Vec::new(),
        placements: Vec::new(),
        model_libraries: Vec::new(),
        problems: Vec::new(),
        notes: Vec::new(),
    };
    let mut windsocks: Vec<LatLon> = Vec::new();
    let mut containers: Vec<RawContainerPlacement> = Vec::new();

    for path in &source.files {
        let rel = path.strip_prefix(&source.root).unwrap_or(path).display().to_string();
        let kinds = match section_kinds(path) {
            Ok(k) => k,
            Err(e) => {
                loaded.problems.push(format!("{rel}: {e}"));
                continue;
            }
        };
        if kinds.contains(&SECTION_MODEL_DATA) {
            loaded.model_libraries.push(path.clone());
        }
        let wanted = kinds
            .iter()
            .any(|k| matches!(*k, SECTION_AIRPORT | SECTION_AIRPORT_ALT | SECTION_SCENERY_OBJECT));
        if !wanted {
            continue;
        }
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                loaded.problems.push(format!("{rel}: {e}"));
                continue;
            }
        };
        let file = match BglFile::parse(&data) {
            Ok(f) => f,
            Err(e) => {
                loaded.problems.push(format!("{rel}: {e}"));
                continue;
            }
        };
        for rec in file.airport_records() {
            match parse_airport(&rec, hint) {
                Ok(raw) if !raw.ident.is_empty() => {
                    let mut ap = model::from_bgl::airport_from_raw(raw, &rel, &source.name);
                    if source.sim == SimKind::Msfs2024 {
                        ap.source.sim = SimKind::Msfs2024;
                    }
                    loaded.airports.push(ap);
                }
                Ok(_) => loaded
                    .problems
                    .push(format!("{rel}: airport record without an identifier skipped")),
                Err(e) => loaded.problems.push(format!("{rel}: airport record unreadable: {e}")),
            }
        }
        let scan = scenery::parse_placements(&file);
        loaded.placements.extend(scan.placements);
        containers.extend(scan.containers);
        windsocks.extend(scan.windsocks.into_iter().map(|(lat, lon)| LatLon::new(lat, lon)));
    }

    if !containers.is_empty() {
        expand_containers(&source.root, &containers, &mut loaded);
    }

    loaded.airports = model::merge::merge_by_ident(std::mem::take(&mut loaded.airports));

    // Name every material the package or the simulator's stock libraries know.
    let mut catalog = stock_materials().clone();
    catalog.load_package(&source.root);
    for ap in &mut loaded.airports {
        let st = materials::resolve_airport(ap, &catalog);
        let total = st.aprons_named + st.aprons_from_tint + st.aprons_defaulted;
        if total > 0 {
            loaded.notes.push(format!(
                "{}: ground materials named {} of {total}, guessed from tint {} ({} textured with stock stand-ins), defaulted {}; {} painted lines named",
                ap.icao, st.aprons_named, st.aprons_from_tint, st.aprons_stand_in_texture, st.aprons_defaulted, st.lines_named
            ));
        }
    }

    // Jetways: each placed jetway model's parked shape, from the package.
    let st = crate::jetways::resolve_airports(&mut loaded.airports, &source.root, &loaded.model_libraries);
    if st.jetways > 0 {
        loaded.notes.push(format!(
            "jetways: {} placed, {} measured from their MSFS models",
            st.jetways, st.measured
        ));
    }

    // Windsocks are free-standing scenery objects; attach each to the nearest
    // airport within 5 km.
    for w in windsocks {
        let nearest = loaded
            .airports
            .iter_mut()
            .map(|a| (inverse(a.datum, w).0, a))
            .filter(|(d, _)| *d < 5_000.0)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, ap)) = nearest {
            ap.windsocks.push(Windsock { pos: w, lit: true });
        }
    }
    loaded
}

/// The package's SimProp container index: container GUID to file. MSFS 2024
/// packages keep it as `simPropContainers.json` (paths relative to the package
/// root), in whatever folder the author chose.
fn container_index(root: &Path) -> std::collections::HashMap<Guid, PathBuf> {
    let mut index = std::collections::HashMap::new();
    for e in WalkDir::new(root).max_depth(4).into_iter().filter_map(Result::ok) {
        if !e.file_name().to_string_lossy().eq_ignore_ascii_case("simpropcontainers.json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        for item in json["content"].as_array().into_iter().flatten() {
            let (Some(g), Some(p)) = (item["guid"].as_str(), item["path"].as_str()) else { continue };
            let Ok(g) = g.parse::<Guid>() else { continue };
            let rel = p.replace(char::from(92u8), "/");
            let mut path = root.join(&rel);
            if !path.is_file() {
                if let Some(parent) = e.path().parent() {
                    path = parent.join(&rel);
                }
            }
            index.insert(g, path);
        }
    }
    index
}

/// Where a container child ends up in the world.
fn child_placement(c: &RawContainerPlacement, k: &ContainerChild) -> RawPlacement {
    let s = c.scale as f64;
    let (ox, oy, oz) = (k.offset[0] as f64 * s, k.offset[1] as f64 * s, k.offset[2] as f64 * s);
    // Offsets are +X right and +Z forward of the container; heading turns
    // clockwise from north.
    let h = (c.heading as f64).to_radians();
    let east = ox * h.cos() + oz * h.sin();
    let north = -ox * h.sin() + oz * h.cos();
    let p = Plane::new(LatLon::new(c.lat, c.lon)).to_latlon(east, north);
    RawPlacement {
        lat: p.lat,
        lon: p.lon,
        alt_m: c.alt_m + oy,
        agl: c.agl,
        pitch: k.pitch,
        bank: k.bank,
        heading: (c.heading + k.heading).rem_euclid(360.0),
        scale: c.scale * k.scale,
        guid: k.model,
    }
}

/// Replace every container placement with placements of its child models.
fn expand_containers(root: &Path, containers: &[RawContainerPlacement], loaded: &mut Loaded) {
    let index = container_index(root);
    let mut cache: std::collections::HashMap<Guid, Option<Vec<ContainerChild>>> = std::collections::HashMap::new();
    let (mut children, mut missing, mut unreadable) = (0usize, 0usize, 0usize);
    for c in containers {
        let parsed = cache.entry(c.container).or_insert_with(|| {
            let path = index.get(&c.container)?;
            let data = std::fs::read(path).ok()?;
            match spb::parse_container(&data) {
                Ok(v) => Some(v),
                Err(e) => {
                    loaded.problems.push(format!("{}: {e}", path.display()));
                    None
                }
            }
        });
        match parsed {
            Some(list) => {
                for k in list.iter() {
                    loaded.placements.push(child_placement(c, k));
                    children += 1;
                }
            }
            None if index.contains_key(&c.container) => unreadable += 1,
            None => missing += 1,
        }
    }
    loaded.notes.push(format!(
        "{} SimProp container placements expanded into {children} objects ({} container files indexed; {missing} placements name a container not in the package, {unreadable} unreadable)",
        containers.len(),
        index.len()
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_children_are_rotated_by_the_container_heading() {
        let c = RawContainerPlacement {
            lat: 41.975517,
            lon: -87.903277,
            alt_m: 1.0,
            agl: true,
            heading: 180.0,
            scale: 1.0,
            container: Guid::NIL,
        };
        let k = ContainerChild {
            model: Guid([1; 16]),
            offset: [-400.0, 5.0, 100.0], // 400 m left and 100 m ahead
            pitch: 0.0,
            bank: 0.0,
            heading: 90.0,
            scale: 1.0,
        };
        let p = child_placement(&c, &k);
        let plane = Plane::new(LatLon::new(c.lat, c.lon));
        let (east, north) = plane.to_xy(LatLon::new(p.lat, p.lon));
        // Facing south, left is east and ahead is south.
        assert!((east - 400.0).abs() < 0.5, "east {east}");
        assert!((north + 100.0).abs() < 0.5, "north {north}");
        assert_eq!(p.heading, 270.0);
        assert_eq!(p.alt_m, 6.0);
        assert!(p.agl);
    }

    #[test]
    fn a_folder_of_packages_expands_to_each_package() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a-airport", "b-airport"] {
            let p = dir.path().join(name);
            std::fs::create_dir_all(p.join("scenery")).unwrap();
            std::fs::write(p.join("manifest.json"), format!("{{\"title\":\"{name} title\"}}")).unwrap();
            std::fs::write(p.join("scenery/x.bgl"), b"nope").unwrap();
        }
        std::fs::create_dir_all(dir.path().join("not-a-package")).unwrap();
        let sources = discover(&dir.path().to_string_lossy()).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].title.as_deref(), Some("a-airport title"));
        assert_eq!(sources[0].files.len(), 1);
        assert_eq!(sources[0].sim, SimKind::Msfs2020);
    }

    #[test]
    fn ktx2_textures_mark_a_2024_package() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), "{}").unwrap();
        std::fs::write(dir.path().join("t.PNG.KTX2"), b"x").unwrap();
        let sources = discover(&dir.path().to_string_lossy()).unwrap();
        assert_eq!(sources[0].sim, SimKind::Msfs2024);
    }

    #[test]
    fn non_bgl_files_are_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), "{}").unwrap();
        std::fs::write(dir.path().join("broken.bgl"), vec![7u8; 200]).unwrap();
        let src = &discover(&dir.path().to_string_lossy()).unwrap()[0];
        let loaded = load(src, None);
        assert!(loaded.airports.is_empty());
        assert_eq!(loaded.problems.len(), 1);
        assert!(loaded.problems[0].contains("not a BGL"), "{:?}", loaded.problems);
    }

    #[test]
    fn missing_inputs_are_errors() {
        assert!(discover("Z:/definitely/not/here").is_err());
        assert!(discover("auto:1999").is_err());
    }
}
