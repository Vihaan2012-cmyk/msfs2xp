//! Reading MSFS jetway models for their parked shape and reach.
//!
//! X-Plane 12 draws its own animated jetways from one apt.dat row each, so
//! what a converted jetway needs from MSFS is where its tunnel points when
//! parked, how long it is, how far it reaches and what it looks like.
//! Asobo's jetway template spells that out: the ModelInfo XML names an IK
//! chain from the rotunda to the cab pivot (`IK_MainHandle`, `Rotation_Base`
//! to `Pivot`), and X limits on its telescoping bones give the reach. The
//! rest pose of that chain in the glTF node tree is the parked jetway. Some
//! variants rename the skeleton (`SKEL_ROTUNDA` to `SKEL_CABIN`) while their
//! XML keeps the template's names; the constraint on `BoneNN` then applies to
//! the NN-th node below the rotunda.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::bgl::guid::Guid;
use crate::bgl::modellib::{ModelCatalog, ModelLibrary};
use crate::model::{Airport, JetwayModel, JetwaySpec};

/// What resolving found, for the conversion notes.
#[derive(Debug, Default, Clone, Copy)]
pub struct JetwayStats {
    pub jetways: usize,
    pub measured: usize,
}

/// Fill in every placed jetway's shape from the package's SimObjects and
/// model libraries.
pub fn resolve_airports(airports: &mut [Airport], root: &Path, model_libraries: &[PathBuf]) -> JetwayStats {
    let mut stats = JetwayStats::default();
    let jetways = || airports.iter().flat_map(|a| &a.parkings).flat_map(|p| &p.jetways);
    if jetways().next().is_none() {
        return stats;
    }
    let needs_libraries = jetways().any(|j| matches!(j.model, JetwayModel::Library(_)));

    let titles = simobject_titles(root);
    let mut folders: HashMap<PathBuf, Option<JetwaySpec>> = HashMap::new();
    // Most detailed glTF stem -> shape, for static copies of animated jetways.
    let mut by_stem: HashMap<String, JetwaySpec> = HashMap::new();
    for dir in titles.values() {
        if folders.contains_key(dir) {
            continue;
        }
        let measured = measure_folder(dir);
        if let Some((spec, stem)) = &measured {
            by_stem.entry(stem.clone()).or_insert(*spec);
        }
        folders.insert(dir.clone(), measured.map(|(spec, _)| spec));
    }

    let mut catalog = ModelCatalog::default();
    if needs_libraries {
        for path in model_libraries {
            if let Ok(lib) = ModelLibrary::open(path) {
                catalog.add(lib);
            }
        }
        // Jetway models the package names but does not carry are stock MSFS
        // models; an MSFS 2020 install keeps those libraries on disk.
        if jetways().any(|j| matches!(&j.model, JetwayModel::Library(g) if catalog.find(g).is_none())) {
            for lib in stock_model_libraries() {
                catalog.add(lib);
            }
        }
    }
    let mut library: HashMap<Guid, Option<JetwaySpec>> = HashMap::new();

    for ap in airports.iter_mut() {
        for p in &mut ap.parkings {
            for j in &mut p.jetways {
                stats.jetways += 1;
                j.spec = match &j.model {
                    JetwayModel::SimObject(title) => titles
                        .get(&title.to_ascii_lowercase())
                        .and_then(|dir| folders.get(dir).copied().flatten()),
                    JetwayModel::Library(guid) => *library
                        .entry(*guid)
                        .or_insert_with(|| library_spec(&catalog, guid, &by_stem)),
                    JetwayModel::Unknown => None,
                };
                if j.spec.is_some() {
                    stats.measured += 1;
                }
            }
        }
    }
    stats
}

/// The model libraries of the MSFS 2020 install's official packages.
fn stock_model_libraries() -> Vec<ModelLibrary> {
    let mut out = Vec::new();
    for official in crate::materials::stock_official_dirs() {
        let Ok(entries) = std::fs::read_dir(&official) else { continue };
        for e in entries.filter_map(Result::ok) {
            if !e.file_name().to_string_lossy().to_ascii_lowercase().contains("modellib") {
                continue;
            }
            for f in walkdir::WalkDir::new(e.path()).into_iter().filter_map(Result::ok) {
                if f.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bgl")) {
                    if let Ok(lib) = ModelLibrary::open(f.path()) {
                        out.push(lib);
                    }
                }
            }
        }
    }
    out
}

/// Every SimObject title in a package and the model folder it uses.
fn simobject_titles(root: &Path) -> HashMap<String, PathBuf> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    let tops = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("simobjects"));
    for top in tops {
        for e in walkdir::WalkDir::new(top.path()).into_iter().filter_map(Result::ok) {
            if !e.file_name().to_string_lossy().eq_ignore_ascii_case("sim.cfg") {
                continue;
            }
            let Ok(bytes) = std::fs::read(e.path()) else { continue };
            let Some(dir) = e.path().parent() else { continue };
            for (title, model) in sim_cfg_entries(&String::from_utf8_lossy(&bytes)) {
                let folder = if model.is_empty() {
                    "model".to_string()
                } else {
                    format!("model.{model}")
                };
                out.entry(title.to_ascii_lowercase()).or_insert_with(|| dir.join(folder));
            }
        }
    }
    out
}

/// `(title, model)` of every `[fltsim.N]` section in a sim.cfg.
fn sim_cfg_entries(text: &str) -> Vec<(String, String)> {
    let mut sections: Vec<(Option<String>, String)> = vec![(None, String::new())];
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if line.starts_with('[') {
            sections.push((None, String::new()));
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches('"').to_string();
        let last = sections.last_mut().expect("starts with one section");
        match key.trim().to_ascii_lowercase().as_str() {
            "title" => last.0 = Some(value),
            "model" => last.1 = value,
            _ => {}
        }
    }
    sections
        .into_iter()
        .filter_map(|(title, model)| Some((title?, model)))
        .collect()
}

/// The parked shape of the jetway in a SimObject model folder, and the stem
/// of its most detailed glTF file ("ini_omdb_jetway_01").
fn measure_folder(dir: &Path) -> Option<(JetwaySpec, String)> {
    let cfg = std::fs::read_to_string(dir.join("model.cfg"))
        .or_else(|_| std::fs::read_to_string(dir.join("model.CFG")))
        .ok()?;
    let xml_name = cfg.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        k.trim().eq_ignore_ascii_case("normal").then(|| v.trim().to_string())
    })?;
    let xml = std::fs::read_to_string(dir.join(&xml_name)).ok()?;
    let xml = xml.trim_start_matches('\u{feff}');
    let doc = roxmltree::Document::parse(xml).ok()?;
    let lod0 = doc
        .descendants()
        .find(|n| n.has_tag_name("LOD"))?
        .attribute("ModelFile")?
        .to_string();
    let bytes = std::fs::read(dir.join(&lod0)).ok()?;
    let gltf = if bytes.starts_with(b"glTF") {
        glb_json(&bytes)?
    } else {
        serde_json::from_slice(&bytes).ok()?
    };
    let (rest_m, reach_m, angle_deg) = measure(xml, &gltf)?;
    let folder = dir.file_name()?.to_string_lossy().to_string();
    let (glass, second_design) = looks(folder.trim_start_matches("model."));
    let stem = Path::new(&lod0).file_stem()?.to_string_lossy().to_ascii_lowercase();
    let stem = stem.strip_suffix("_lod0").unwrap_or(&stem).to_string();
    Some((
        JetwaySpec {
            rest_m,
            reach_m,
            angle_deg,
            glass,
            second_design,
        },
        stem,
    ))
}

/// The shape of a jetway placed as a library model: measured from its own
/// XML and glTF when it is animated, or borrowed from the animated SimObject
/// it is a static copy of ("INI_OMDB_JETWAY_01_static").
fn library_spec(catalog: &ModelCatalog, guid: &Guid, by_stem: &HashMap<String, JetwaySpec>) -> Option<JetwaySpec> {
    let lib = catalog.find(guid)?;
    let name = lib.info(guid).ok()?.name;
    let (glass, second_design) = looks(&name);
    let xml = lib.xml(guid).ok().flatten();
    let gltf = lib.load_lod(guid, 0).ok().and_then(|b| glb_json(&b));
    if let (Some(xml), Some(gltf)) = (xml, gltf) {
        if let Some((rest_m, reach_m, angle_deg)) = measure(&xml, &gltf) {
            return Some(JetwaySpec {
                rest_m,
                reach_m,
                angle_deg,
                glass,
                second_design,
            });
        }
    }
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix("_static").unwrap_or(&lower);
    by_stem.get(stem).map(|s| JetwaySpec {
        glass,
        second_design,
        ..*s
    })
}

/// The JSON chunk of a GLB file.
fn glb_json(glb: &[u8]) -> Option<Value> {
    if glb.len() < 20 || !glb.starts_with(b"glTF") {
        return None;
    }
    let len = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
    let json = glb.get(20..20 + len)?;
    let end = json.iter().rposition(|&c| c != 0 && c != b' ').map_or(0, |p| p + 1);
    serde_json::from_slice(&json[..end]).ok()
}

/// Glass tunnel and second cab design, from a model or title name: `_g` or
/// "glass" for glass; `jetway2`, `jetway_02` or a leading `02` for the
/// second design.
fn looks(name: &str) -> (bool, bool) {
    let n = name.to_ascii_lowercase();
    let words: Vec<&str> = n
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let glass = words.iter().any(|w| *w == "g" || w.contains("glass"));
    let second = n.contains("jetway2") || n.contains("jetway_02") || words.first() == Some(&"02");
    (glass, second)
}

/// Parked length, (shortest, longest) reach and parked direction of a
/// jetway's main IK chain. The direction is in degrees from the model's +Z
/// (forward) towards +X (its left).
fn measure(xml: &str, gltf: &Value) -> Option<(f32, (f32, f32), f32)> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let child_text = |node: roxmltree::Node, tag: &str| {
        node.children()
            .find(|c| c.has_tag_name(tag))
            .and_then(|c| c.text())
            .map(|t| t.trim().to_string())
    };
    let (start, end) = doc
        .descendants()
        .find(|n| n.has_tag_name("IKChain") && n.attribute("Name") == Some("IK_MainHandle"))
        .and_then(|c| Some((child_text(c, "Start")?, child_text(c, "End")?)))
        .unwrap_or_default();
    // Telescoping bones: node name, shortest and longest offset along X.
    let mut limits: Vec<(String, f64, f64)> = Vec::new();
    for c in doc.descendants().filter(|n| n.has_tag_name("IKConstraint")) {
        let Some(node) = child_text(c, "Node") else { continue };
        let Some(x) = c.children().find(|n| n.has_tag_name("X")) else { continue };
        let num = |a: &str| x.attribute(a).and_then(|v| v.parse::<f64>().ok());
        if let (Some(lo), Some(hi)) = (num("min"), num("max")) {
            limits.push((node, lo, hi));
        }
    }

    let nodes = gltf["nodes"].as_array()?;
    let find = |name: &str| nodes.iter().position(|n| n["name"].as_str() == Some(name));
    let (s, e) = match (find(&start), find(&end)) {
        (Some(s), Some(e)) => (s, e),
        _ => (find("SKEL_ROTUNDA")?, find("SKEL_CABIN")?),
    };
    let mut parent: HashMap<usize, usize> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        for c in n["children"].as_array().into_iter().flatten().filter_map(Value::as_u64) {
            parent.insert(c as usize, i);
        }
    }
    // The chain from the rotunda down to the cab.
    let mut chain = vec![e];
    let mut at = e;
    while at != s {
        at = *parent.get(&at)?;
        chain.push(at);
        if chain.len() > 64 {
            return None;
        }
    }
    chain.reverse();

    let (ps, pe) = (world_position(nodes, &parent, s), world_position(nodes, &parent, e));
    let (dx, dz) = (pe[0] - ps[0], pe[2] - ps[2]);
    let rest = dx.hypot(dz);
    if rest < 1.0 {
        return None;
    }
    let angle = dx.atan2(dz).to_degrees();
    let (mut lo, mut hi) = (rest, rest);
    for (name, min, max) in &limits {
        let bone = find(name).filter(|i| chain.contains(i)).or_else(|| {
            let n: usize = name.strip_prefix("Bone")?.parse().ok()?;
            chain.get(n).copied().filter(|&i| i != e)
        });
        let Some(i) = bone else { continue };
        let x = nodes[i]["translation"][0].as_f64().unwrap_or(0.0);
        lo += min - x;
        hi += max - x;
    }
    Some((rest as f32, (lo.min(rest) as f32, hi.max(rest) as f32), angle as f32))
}

fn vec3(v: &Value, default: f64) -> [f64; 3] {
    let g = |i: usize| v[i].as_f64().unwrap_or(default);
    [g(0), g(1), g(2)]
}

fn qmul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let r = qmul(qmul(q, [v[0], v[1], v[2], 0.0]), [-q[0], -q[1], -q[2], q[3]]);
    [r[0], r[1], r[2]]
}

/// A node's position in model space, from the translation, rotation and
/// scale of it and its ancestors.
fn world_position(nodes: &[Value], parent: &HashMap<usize, usize>, i: usize) -> [f64; 3] {
    let mut path = vec![i];
    let mut at = i;
    while let Some(&p) = parent.get(&at) {
        if path.len() > 64 {
            break;
        }
        path.push(p);
        at = p;
    }
    let (mut pos, mut rot, mut scale) = ([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3]);
    for &j in path.iter().rev() {
        let n = &nodes[j];
        let t = vec3(&n["translation"], 0.0);
        let r = n["rotation"].as_array().map_or([0.0, 0.0, 0.0, 1.0], |_| {
            let g = |k: usize| n["rotation"][k].as_f64().unwrap_or(if k == 3 { 1.0 } else { 0.0 });
            [g(0), g(1), g(2), g(3)]
        });
        let s = vec3(&n["scale"], 1.0);
        let v = rotate(rot, [t[0] * scale[0], t[1] * scale[1], t[2] * scale[2]]);
        pos = [pos[0] + v[0], pos[1] + v[1], pos[2] + v[2]];
        rot = qmul(rot, r);
        scale = [scale[0] * s[0], scale[1] * s[1], scale[2] * s[2]];
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE_XML: &str = r#"<ModelInfo>
        <IKChain Name="IK_MainHandle"><Start>Rotation_Base</Start><End>Pivot</End></IKChain>
        <IKConstraint><Node>Bone01</Node><Bank min="-6" max="6"/></IKConstraint>
        <IKConstraint><Node>Bone02</Node><X min="0.6" max="9.0"/></IKConstraint>
        <IKConstraint><Node>Bone03</Node><X min="0.4" max="9.9"/></IKConstraint>
        <IKConstraint><Node>Bone08</Node><X/></IKConstraint>
    </ModelInfo>"#;

    /// O'Hare's jetway1_b chain, under the given node names.
    fn chain(names: [&str; 5]) -> Value {
        serde_json::json!({ "nodes": [
            { "name": "ROOT", "children": [1] },
            { "name": names[0], "translation": [0.0, 5.3, 0.0], "children": [2] },
            { "name": names[1], "translation": [1.32, 0.0, 0.0], "children": [3] },
            { "name": names[2], "translation": [0.97, 0.0, 0.0], "children": [4] },
            { "name": names[3], "translation": [1.98, 0.0, 0.0], "children": [5] },
            { "name": names[4], "translation": [12.24, -1.38, 0.0] }
        ]})
    }

    fn assert_jetway1(m: (f32, (f32, f32), f32)) {
        let (rest, (lo, hi), angle) = m;
        assert!((rest - 16.51).abs() < 0.01, "rest {rest}");
        assert!((lo - 14.56).abs() < 0.01 && (hi - 32.46).abs() < 0.01, "reach {lo}-{hi}");
        assert!((angle - 90.0).abs() < 0.01, "the tunnel lies along +X");
    }

    #[test]
    fn measures_the_template_chain() {
        let g = chain(["Rotation_Base", "Bone01", "Bone02", "Bone03", "Pivot"]);
        assert_jetway1(measure(TEMPLATE_XML, &g).unwrap());
    }

    #[test]
    fn renamed_skeletons_map_template_bones_by_position() {
        let g = chain(["SKEL_ROTUNDA", "SKEL_TUNNEL_01", "SKEL_TUNNEL_02", "SKEL_TUNNEL_03", "SKEL_CABIN"]);
        assert_jetway1(measure(TEMPLATE_XML, &g).unwrap());
    }

    #[test]
    fn a_turned_rotunda_turns_the_tunnel() {
        let mut g = chain(["Rotation_Base", "Bone01", "Bone02", "Bone03", "Pivot"]);
        // 90 degrees about +Y turns +X into -Z.
        let h = std::f64::consts::FRAC_1_SQRT_2;
        g["nodes"][1]["rotation"] = serde_json::json!([0.0, h, 0.0, h]);
        let (rest, _, angle) = measure(TEMPLATE_XML, &g).unwrap();
        assert!((rest - 16.51).abs() < 0.01);
        assert!((angle.abs() - 180.0).abs() < 0.01, "angle {angle}");
    }

    #[test]
    #[ignore = "needs the O'Hare and Dubai test packages"]
    fn measures_the_real_packages() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("Test Airports");
        for dir in [
            "inibuilds-airport-kord-chicago/SimObjects/Landmarks/kord-jetways/model.jetway1_b",
            "inibuilds-airport-kord-chicago/SimObjects/Landmarks/kord-jetways/model.jetway1_b_ini",
            "inibuilds-airport-omdb-dubai/SimObjects/Landmarks/OMDB_jetways/model.01",
            "inibuilds-airport-omdb-dubai/SimObjects/Landmarks/OMDB_jetways/model.02_ini",
        ] {
            let d = root.join(dir);
            let cfg = std::fs::read_to_string(d.join("model.cfg")).map(|c| c.len());
            println!("{dir}: cfg {cfg:?} -> {:?}", measure_folder(&d));
        }
        let titles = simobject_titles(&root.join("inibuilds-airport-omdb-dubai"));
        println!("OMDB titles: {}", titles.len());
    }

    #[test]
    fn names_say_glass_and_cab_design() {
        assert_eq!(looks("jetway1_g_short_nologo"), (true, false));
        assert_eq!(looks("jetway2_b"), (false, true));
        assert_eq!(looks("02_long_ini"), (false, true));
        assert_eq!(looks("INI_OMDB_JETWAY_01_static"), (false, false));
    }

    #[test]
    fn sim_cfg_sections_give_titles_and_models() {
        let cfg = "[fltsim.0]\ntitle=KORD Jetway B1x\nmodel=jetway1_b\ntexture=B1x\n\n[fltsim.1]\ntitle=\"Plain\" // quoted\n\n[General]\ncategory=StaticObject\n";
        assert_eq!(
            sim_cfg_entries(cfg),
            vec![
                ("KORD Jetway B1x".to_string(), "jetway1_b".to_string()),
                ("Plain".to_string(), String::new())
            ]
        );
    }
}
