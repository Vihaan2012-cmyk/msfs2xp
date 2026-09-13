//! The building stage: placed MSFS models to X-Plane objects and DSF tiles.
//!
//! For every library-object placement in a package: find its model by GUID in
//! the package's model libraries, inflate its most detailed LOD, convert it to
//! OBJ8 (one object per texture), convert the textures it uses, and place the
//! objects in DSF tiles. Models are converted once per distinct scale, since a
//! DSF placement cannot scale an object.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rayon::prelude::*;
use walkdir::WalkDir;

use crate::bgl::guid::Guid;
use crate::bgl::modellib::{ModelCatalog, ModelLibrary};
use crate::bgl::records::scenery::RawPlacement;
use crate::model3d::{load_glb, split_by_texture, write_obj8_lods, LodPart, ObjOptions};
use crate::package::Loaded;
use crate::texture;
use crate::decals::{self, DecalKind};
use crate::xplane::dsf::{self, DrapedPolygon, Placement};

/// What the building stage did.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ObjectsReport {
    pub placements: usize,
    pub placed: usize,
    pub models_converted: usize,
    pub object_files: usize,
    pub textures_written: usize,
    pub triangles: usize,
    /// Placements whose model was found in neither the package nor a stock
    /// library on disk (usually MSFS 2024 stock objects, which are streamed).
    pub not_in_package: usize,
    /// Placements whose model came from an MSFS 2020 stock library.
    pub stock_placements: usize,
    /// The most placed of those models: (GUID, placements), most first.
    pub missing_models: Vec<(String, usize)>,
    pub failed_models: Vec<String>,
    pub missing_textures: Vec<String>,
    /// Textured apron markings draped over the pavement.
    pub decals: usize,
    pub missing_decal_textures: Vec<String>,
    pub dsf_tiles: Vec<String>,
}

/// Options for the building stage.
#[derive(Debug, Clone, Copy)]
pub struct ObjectOptions {
    /// Which LOD to start from; 0 is the most detailed.
    pub lod: usize,
    /// Step down to coarser LODs until a model has at most this many triangles.
    pub max_triangles: usize,
}

impl Default for ObjectOptions {
    fn default() -> Self {
        ObjectOptions {
            lod: 0,
            max_triangles: 500_000,
        }
    }
}

/// A file-name-safe stem.
fn safe(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if s.is_empty() {
        "model".into()
    } else {
        s
    }
}

/// Every texture file in the package, keyed by lower-case file name.
fn texture_index(root: &Path) -> HashMap<String, PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            (name.ends_with(".ktx2") || name.ends_with(".dds") || name.ends_with(".png"))
                .then(|| (name, e.path().to_path_buf()))
        })
        .collect()
}

/// The output stem for a texture: its name without the MSFS double extension.
fn texture_stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let mut stem = name.to_string();
    for ext in [".ktx2", ".dds", ".png", ".tif", ".jpg"] {
        if lower.ends_with(ext) {
            stem.truncate(stem.len() - ext.len());
            let l = stem.to_ascii_lowercase();
            for inner in [".png", ".tif", ".jpg", ".dds"] {
                if l.ends_with(inner) {
                    stem.truncate(stem.len() - inner.len());
                    break;
                }
            }
            break;
        }
    }
    safe(&stem)
}

/// One converted object variant: model, scale in thousandths, height above
/// ground in quarter metres.
type VariantKey = (Guid, i32, i32);

/// The object files written for one variant and its triangle count, or why it failed.
type ModelOutcome = Result<(Vec<String>, usize), String>;

/// Delete `.dsf` tiles left by an earlier run (apt.dat is kept).
/// Draw distances in metres for a model: full detail out to the first, the
/// coarse level out to the second, nothing beyond. Scaled to the model's size
/// the way MSFS picks levels by screen size: 400 radii away a model spans a
/// few pixels, so props fade out while large buildings (50 m and up) never do.
fn draw_distances(model: &crate::model3d::glb::Model, scale: f32) -> (f32, f32) {
    let radius = model.bounds().map_or(1.0, |(lo, hi)| {
        let diagonal: f32 = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f32>().sqrt();
        diagonal / 2.0 * if scale > 0.0 { scale } else { 1.0 }
    });
    let far = if radius >= 50.0 {
        f32::INFINITY
    } else {
        (radius * 400.0).clamp(300.0, 20_000.0).round()
    };
    let near = (radius * 40.0).clamp(30.0, far.min(100_000.0) / 2.0).round();
    (near, far)
}

fn remove_dsf_tiles(nav: &Path) -> std::io::Result<()> {
    if !nav.is_dir() {
        return Ok(());
    }
    for e in WalkDir::new(nav).min_depth(2).max_depth(2).into_iter().filter_map(Result::ok) {
        if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("dsf")) {
            std::fs::remove_file(e.path())?;
        }
    }
    Ok(())
}

/// Convert the placed models of a loaded package into `pack_dir`.
pub fn build(loaded: &Loaded, pack_dir: &Path, opts: &ObjectOptions) -> anyhow::Result<ObjectsReport> {
    let mut report = ObjectsReport {
        placements: loaded.placements.len(),
        ..Default::default()
    };
    let decal_list: Vec<decals::DecalPolygon> =
        loaded.airports.iter().flat_map(|a| decals::airport_decals(&a.aprons)).collect();
    if loaded.placements.is_empty() && decal_list.is_empty() {
        return Ok(report);
    }
    let mut catalog = ModelCatalog::default();
    let mut package_guids: HashSet<Guid> = HashSet::new();
    for path in &loaded.model_libraries {
        match ModelLibrary::open(path) {
            Ok(lib) => {
                package_guids.extend(lib.guids().iter().copied());
                catalog.add(lib)
            }
            Err(e) => report.failed_models.push(format!("{}: {e}", path.display())),
        }
    }
    // Models the package does not carry are stock MSFS library objects. MSFS
    // 2024 streams its own, but an MSFS 2020 install keeps the shared generic
    // libraries (hangars, buildings, props) on disk, so use those when present.
    // They are added after the package's libraries, which therefore win.
    // Stock packages: model libraries (for placed stock models) and material
    // libraries, whose textures some decals use (O'Hare's asphalt decals point
    // at Asobo's shared DECALASPHALT02 texture).
    let mut stock_dirs: Vec<PathBuf> = Vec::new();
    let mut stock_texture_dirs: Vec<PathBuf> = Vec::new();
    for official in crate::materials::stock_official_dirs() {
        let Ok(entries) = std::fs::read_dir(&official) else { continue };
        for e in entries.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if name.contains("modellib") {
                stock_dirs.push(e.path());
                stock_texture_dirs.push(e.path());
            } else if name.contains("material") {
                stock_texture_dirs.push(e.path());
            }
        }
    }
    if loaded.placements.iter().any(|p| !package_guids.contains(&p.guid)) {
        for dir in &stock_dirs {
            for f in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
                if f.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) {
                    if let Ok(lib) = ModelLibrary::open(f.path()) {
                        catalog.add(lib);
                    }
                }
            }
        }
    }

    // Distinct (model, scale, height) triples. Scale is rounded so float noise
    // does not multiply the object count. DSF objects always sit on the
    // terrain, so MSFS heights above ground are baked into the object, in
    // quarter-metre steps; anything lower than that stays on the ground.
    // Heights MSFS gives relative to sea level become heights above the field
    // elevation of an airport within 10 km, whose terrain X-Plane flattens to
    // that elevation. Elsewhere they cannot be placed and stay on the ground.
    let airports: Vec<(crate::geo::LatLon, f64)> =
        loaded.airports.iter().map(|a| (a.datum, a.elevation_m)).collect();
    let height_of = |p: &RawPlacement| -> f64 {
        if !p.alt_m.is_finite() {
            return 0.0;
        }
        if p.agl {
            return p.alt_m;
        }
        let here = crate::geo::LatLon::new(p.lat, p.lon);
        airports
            .iter()
            .find(|(datum, _)| crate::geo::inverse(*datum, here).0 < 10_000.0)
            .map(|(_, elevation)| p.alt_m - elevation)
            .unwrap_or(0.0)
    };
    let key = |p: &RawPlacement| -> VariantKey {
        let h = height_of(p);
        let quarters = if h.abs() >= 0.25 { (h * 4.0).round() as i32 } else { 0 };
        (p.guid, (p.scale * 1000.0).round() as i32, quarters)
    };
    let mut wanted: BTreeMap<VariantKey, f32> = BTreeMap::new();
    let mut missing: HashMap<Guid, usize> = HashMap::new();
    for p in &loaded.placements {
        if catalog.find(&p.guid).is_some() {
            wanted.entry(key(p)).or_insert(p.scale);
            if !package_guids.contains(&p.guid) {
                report.stock_placements += 1;
            }
        } else {
            report.not_in_package += 1;
            *missing.entry(p.guid).or_default() += 1;
        }
    }
    let mut missing: Vec<(String, usize)> = missing.into_iter().map(|(g, n)| (g.to_string(), n)).collect();
    missing.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    missing.truncate(200);
    report.missing_models = missing;

    let objects_dir = pack_dir.join("objects");
    let textures_dir = objects_dir.join("textures");
    // Start clean: object names depend on scale and height, so files from an
    // earlier run would otherwise linger and bloat the pack.
    if objects_dir.is_dir() {
        std::fs::remove_dir_all(&objects_dir)?;
    }
    remove_dsf_tiles(&pack_dir.join("Earth nav data"))?;
    std::fs::create_dir_all(&textures_dir)?;
    // Stock textures first, then the package's own, so the package wins.
    let mut textures: HashMap<String, PathBuf> = HashMap::new();
    for dir in &stock_texture_dirs {
        textures.extend(texture_index(dir));
    }
    textures.extend(texture_index(&loaded.source.root));
    let written_textures: Mutex<HashMap<String, Option<String>>> = Mutex::new(HashMap::new());
    let missing: Mutex<HashSet<String>> = Mutex::new(HashSet::new());

    // Convert (or reuse) a texture, returning its path relative to the objects.
    let texture_for = |name: &str| -> Option<String> {
        let lower = name.to_ascii_lowercase();
        if let Some(done) = written_textures.lock().ok()?.get(&lower) {
            return done.clone();
        }
        let result = textures.get(&lower).and_then(|src| {
            let data = std::fs::read(src).ok()?;
            let converted = texture::convert_for_xplane(&data).ok()?;
            let file = format!("{}.{}", texture_stem(name), converted.extension);
            std::fs::write(textures_dir.join(&file), &converted.bytes).ok()?;
            Some(format!("textures/{file}"))
        });
        if result.is_none() {
            if let Ok(mut m) = missing.lock() {
                m.insert(name.to_string());
            }
        }
        if let Ok(mut w) = written_textures.lock() {
            w.insert(lower, result.clone());
        }
        result
    };

    // Convert models in parallel. Each yields the object files it produced.
    let results: Vec<(VariantKey, ModelOutcome)> = wanted
        .par_iter()
        .map(|(&k, &scale)| {
            let (guid, _, quarters) = k;
            let lib = catalog.find(&guid).expect("filtered above");
            let outcome = (|| -> Result<(Vec<String>, usize), String> {
                let info = lib.info(&guid).map_err(|e| e.to_string())?;
                let last = info.lods.len().saturating_sub(1);
                // Start at the requested LOD and step down until the model fits
                // the triangle budget: MSFS LOD0 of a landmark can exceed a
                // million triangles, far more than X-Plane should draw per object.
                let mut lod = opts.lod.min(last);
                let model = loop {
                    let glb = lib.load_lod(&guid, lod).map_err(|e| e.to_string())?;
                    let m = load_glb(&glb).map_err(|e| e.to_string())?;
                    if m.triangle_count() <= opts.max_triangles || lod >= last {
                        break m;
                    }
                    lod += 1;
                };
                // A coarser MSFS level for distant views: the first with at most a
                // third of the triangles (MSFS roughly halves them per level).
                let base_tris = model.triangle_count();
                let mut far_model = None;
                for l in lod + 1..=last {
                    let Some(m) = lib.load_lod(&guid, l).ok().and_then(|b| load_glb(&b).ok()) else {
                        break;
                    };
                    let t = m.triangle_count();
                    if t > 0 && (t * 3 <= base_tris || (l == last && t * 10 <= base_tris * 7)) {
                        far_model = Some(m);
                        break;
                    }
                }
                let (near_to, draw_to) = draw_distances(&model, scale);
                let base = if info.name.is_empty() { guid.to_string() } else { info.name.clone() };
                let mut suffix = if (scale - 1.0).abs() > 1e-3 { format!("_s{}", k.1) } else { String::new() };
                if quarters != 0 {
                    suffix.push_str(&format!("_h{quarters}"));
                }
                let offset_y = quarters as f32 / 4.0;
                let mut files = Vec::new();
                // One object per texture pair, holding that pair's meshes from both
                // levels of detail.
                let near_groups = split_by_texture(&model);
                let far_groups = far_model.as_ref().map(split_by_texture).unwrap_or_default();
                let mut keys: Vec<(Option<String>, Option<String>)> = Vec::new();
                for g in near_groups.iter().chain(&far_groups) {
                    let k = (g.0.clone(), g.1.clone());
                    if !keys.contains(&k) {
                        keys.push(k);
                    }
                }
                if keys.is_empty() && !model.lights.is_empty() {
                    keys.push((None, None)); // a lights-only object
                }
                let meshes_of = |groups: &[(Option<String>, Option<String>, Vec<usize>)], k: &(Option<String>, Option<String>)| {
                    groups
                        .iter()
                        .find(|g| g.0 == k.0 && g.1 == k.1)
                        .map(|g| g.2.clone())
                        .unwrap_or_default()
                };
                for (i, key) in keys.iter().enumerate() {
                    let texture = key.0.as_deref().and_then(&texture_for);
                    let texture_lit = key.1.as_deref().and_then(&texture_for);
                    let options = ObjOptions {
                        texture,
                        scale,
                        offset_y,
                        texture_lit,
                        lights: i == 0,
                    };
                    let mut parts = vec![LodPart {
                        model: &model,
                        meshes: meshes_of(&near_groups, key),
                        near: 0.0,
                        far: draw_to,
                    }];
                    if let Some(far) = &far_model {
                        parts[0].far = near_to;
                        parts.push(LodPart {
                            model: far,
                            meshes: meshes_of(&far_groups, key),
                            near: near_to,
                            far: draw_to,
                        });
                    }
                    let text = write_obj8_lods(&parts, &options);
                    let file = format!("{}{}_{}.obj", safe(&base), suffix, i);
                    std::fs::write(objects_dir.join(&file), text).map_err(|e| e.to_string())?;
                    files.push(format!("objects/{file}"));
                }
                Ok((files, model.triangle_count()))
            })();
            (k, outcome.map_err(|e| format!("{guid}: {e}")))
        })
        .collect();

    let mut object_paths: Vec<String> = Vec::new();
    let mut objects_of: HashMap<VariantKey, Vec<usize>> = HashMap::new();
    for (k, r) in results {
        match r {
            Ok((files, tris)) => {
                report.models_converted += 1;
                report.triangles += tris;
                let idx: Vec<usize> = files
                    .into_iter()
                    .map(|f| {
                        object_paths.push(f);
                        object_paths.len() - 1
                    })
                    .collect();
                objects_of.insert(k, idx);
            }
            Err(e) => report.failed_models.push(e),
        }
    }
    report.object_files = object_paths.len();

    let mut placements = Vec::new();
    for p in &loaded.placements {
        if let Some(objs) = objects_of.get(&key(p)) {
            report.placed += 1;
            for &o in objs {
                placements.push(Placement {
                    lat: p.lat,
                    lon: p.lon,
                    heading: p.heading as f64,
                    object: o,
                });
            }
        }
    }

    // Decals: convert each texture once, write one .pol per texture, placement
    // mode and layer, and drape the polygons in the package's drawing order.
    let decal_dir = objects_dir.join("decals");
    let mut polygon_defs: Vec<String> = Vec::new();
    let mut def_of: HashMap<(String, bool, bool), usize> = HashMap::new();
    let mut draped: Vec<DrapedPolygon> = Vec::new();
    let mut missing_decals: HashSet<String> = HashSet::new();
    for d in &decal_list {
        let grime = d.kind == DecalKind::Grime;
        let key = (d.texture.to_ascii_lowercase(), d.stretched, grime);
        let def = match def_of.get(&key) {
            Some(&i) => Some(i),
            None => match texture_for(&d.texture) {
                Some(rel) => {
                    std::fs::create_dir_all(&decal_dir)?;
                    let file = format!(
                        "{}_{}_{}.pol",
                        texture_stem(&d.texture),
                        if d.stretched { "fit" } else { "tile" },
                        if grime { "grime" } else { "mark" }
                    );
                    std::fs::write(decal_dir.join(&file), decals::pol_text(&format!("../{rel}"), d.stretched, d.kind))?;
                    polygon_defs.push(format!("objects/decals/{file}"));
                    def_of.insert(key, polygon_defs.len() - 1);
                    Some(polygon_defs.len() - 1)
                }
                None => {
                    missing_decals.insert(d.texture.clone());
                    None
                }
            },
        };
        if let Some(def) = def {
            draped.push(DrapedPolygon {
                def,
                points: d.points.clone(),
            });
        }
    }
    report.decals = draped.len();
    let mut missing_decals: Vec<String> = missing_decals.into_iter().collect();
    missing_decals.sort();
    report.missing_decal_textures = missing_decals;

    let nav = pack_dir.join("Earth nav data");
    for (rel, bytes) in dsf::build_tiles(&object_paths, &placements, &polygon_defs, &draped, "msfs2xp") {
        let path = nav.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes)?;
        report.dsf_tiles.push(rel.display().to_string());
    }
    report.textures_written = written_textures
        .lock()
        .map(|w| w.values().filter(|v| v.is_some()).count())
        .unwrap_or(0);
    let mut missing: Vec<String> = missing.into_inner().unwrap_or_default().into_iter().collect();
    missing.sort();
    report.missing_textures = missing;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_stems_drop_the_double_extension() {
        assert_eq!(texture_stem("OMDB_GATE_MARKERS_ALBD.PNG.KTX2"), "OMDB_GATE_MARKERS_ALBD");
        assert_eq!(texture_stem("AC UNIT_COMP.PNG.KTX2"), "AC_UNIT_COMP");
        assert_eq!(texture_stem("plain.dds"), "plain");
        assert_eq!(texture_stem("weird"), "weird");
    }

    #[test]
    fn draw_distance_follows_model_size() {
        use crate::model3d::glb::{Mesh, Model, Vertex};
        let sized = |half: f32| Model {
            meshes: vec![Mesh {
                vertices: [[-half, 0.0, 0.0], [half, 0.0, 0.0]]
                    .iter()
                    .map(|&pos| Vertex {
                        pos,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(draw_distances(&sized(1.0), 1.0), (40.0, 400.0), "a person");
        assert_eq!(draw_distances(&sized(0.3), 1.0), (30.0, 300.0), "small props keep a floor");
        assert_eq!(draw_distances(&sized(1.0), 2.0), (80.0, 800.0), "scale counts");
        assert!(draw_distances(&sized(200.0), 1.0).1.is_infinite(), "terminals never fade");
    }

    #[test]
    fn names_are_filesystem_safe() {
        assert_eq!(safe("OMDB Gate/B12"), "OMDB_Gate_B12");
        assert_eq!(safe(""), "model");
    }
}
