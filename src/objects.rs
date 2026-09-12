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
use crate::model3d::{load_glb, split_by_texture, write_obj8, ObjOptions};
use crate::package::Loaded;
use crate::texture;
use crate::xplane::dsf::{self, Placement};

/// What the building stage did.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ObjectsReport {
    pub placements: usize,
    pub placed: usize,
    pub models_converted: usize,
    pub object_files: usize,
    pub textures_written: usize,
    pub triangles: usize,
    /// Placements whose model is not in the package (stock MSFS library objects).
    pub not_in_package: usize,
    pub failed_models: Vec<String>,
    pub missing_textures: Vec<String>,
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
            max_triangles: 100_000,
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
    if loaded.placements.is_empty() {
        return Ok(report);
    }
    let mut catalog = ModelCatalog::default();
    for path in &loaded.model_libraries {
        match ModelLibrary::open(path) {
            Ok(lib) => catalog.add(lib),
            Err(e) => report.failed_models.push(format!("{}: {e}", path.display())),
        }
    }

    // Distinct (model, scale, height) triples. Scale is rounded so float noise
    // does not multiply the object count. DSF objects always sit on the
    // terrain, so MSFS heights above ground are baked into the object, in
    // quarter-metre steps; anything lower than that stays on the ground.
    let key = |p: &RawPlacement| -> VariantKey {
        let quarters = if p.agl && p.alt_m.is_finite() && p.alt_m.abs() >= 0.25 {
            (p.alt_m * 4.0).round() as i32
        } else {
            0
        };
        (p.guid, (p.scale * 1000.0).round() as i32, quarters)
    };
    let mut wanted: BTreeMap<VariantKey, f32> = BTreeMap::new();
    for p in &loaded.placements {
        if catalog.find(&p.guid).is_some() {
            wanted.entry(key(p)).or_insert(p.scale);
        } else {
            report.not_in_package += 1;
        }
    }

    let objects_dir = pack_dir.join("objects");
    let textures_dir = objects_dir.join("textures");
    // Start clean: object names depend on scale and height, so files from an
    // earlier run would otherwise linger and bloat the pack.
    if objects_dir.is_dir() {
        std::fs::remove_dir_all(&objects_dir)?;
    }
    remove_dsf_tiles(&pack_dir.join("Earth nav data"))?;
    std::fs::create_dir_all(&textures_dir)?;
    let textures = texture_index(&loaded.source.root);
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
                let base = if info.name.is_empty() { guid.to_string() } else { info.name.clone() };
                let mut suffix = if (scale - 1.0).abs() > 1e-3 { format!("_s{}", k.1) } else { String::new() };
                if quarters != 0 {
                    suffix.push_str(&format!("_h{quarters}"));
                }
                let offset_y = quarters as f32 / 4.0;
                let mut files = Vec::new();
                for (i, (tex, meshes)) in split_by_texture(&model).into_iter().enumerate() {
                    let texture = tex.as_deref().and_then(&texture_for);
                    let text = write_obj8(&model, &meshes, &ObjOptions { texture, scale, offset_y });
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

    let nav = pack_dir.join("Earth nav data");
    for (rel, bytes) in dsf::build_tiles(&object_paths, &placements, "msfs2xp") {
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
    fn names_are_filesystem_safe() {
        assert_eq!(safe("OMDB Gate/B12"), "OMDB_Gate_B12");
        assert_eq!(safe(""), "model");
    }
}
