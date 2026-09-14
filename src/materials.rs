//! MSFS material libraries: what a material GUID looks like.
//!
//! MSFS 2024 ground polygons and painted lines no longer carry a meaningful
//! surface code. They reference a material by GUID, and the material library
//! (`MaterialLibs/*/Library.xml` in the package, plus the simulator's stock
//! libraries) gives it a name such as `INI_Asphalt_1`, `INI_Number_3` or
//! `INI_Lines_Dashed_White`. The name is the best available description of what
//! the polygon looks like, which is what X-Plane needs.
//!
//! The library's `SurfaceType` attribute is the physics surface (for sounds and
//! friction), not the look: iniBuilds' `INI_Asphalt_1` is tagged CONCRETE. So
//! names are consulted first and `SurfaceType` only as a fallback.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::bgl::guid::Guid;
use crate::model::{Airport, Surface};
#[cfg(test)]
use crate::model::Apron;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialInfo {
    pub name: String,
    pub surface_type: String,
    /// File name of the decal texture (binding MTL_BITMAP_DECAL0), if any.
    pub decal_texture: Option<String>,
    /// Where that texture is on disk, when the library was read from a file.
    pub texture_path: Option<PathBuf>,
}

/// Mean brightness (0..=255) of a material's colour texture, from a small mip
/// level. This is what separates iniBuilds' dark taxiway asphalt from its
/// light concrete tiles, and X-Plane 12 has pavement variants for both.
pub fn brightness(info: &MaterialInfo) -> Option<u8> {
    let data = std::fs::read(info.texture_path.as_ref()?).ok()?;
    let (_, _, rgba) = crate::texture::decode_within(&data, 32).ok()?;
    let (mut sum, mut n) = (0.0f64, 0u32);
    for p in rgba.chunks_exact(4).filter(|p| p[3] >= 8) {
        sum += 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64;
        n += 1;
    }
    (n > 0).then(|| (sum / n as f64).round() as u8)
}

/// Every material we could find, keyed by GUID.
#[derive(Debug, Default, Clone)]
pub struct MaterialCatalog {
    map: HashMap<Guid, MaterialInfo>,
}

impl MaterialCatalog {
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn get(&self, guid: &Guid) -> Option<&MaterialInfo> {
        self.map.get(guid)
    }

    pub fn insert(&mut self, guid: Guid, info: MaterialInfo) {
        self.map.entry(guid).or_insert(info);
    }

    /// Every material with this name, ignoring case (stock libraries repeat
    /// names with different textures).
    pub fn find_by_name<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a MaterialInfo> + 'a {
        self.map.values().filter(move |m| m.name.eq_ignore_ascii_case(name))
    }

    /// Parse one `Library.xml`. Returns how many materials it defined.
    pub fn load_library_text(&mut self, xml: &str) -> usize {
        let found = parse_library(xml);
        let n = found.len();
        for (guid, info) in found {
            self.insert(guid, info);
        }
        n
    }

    /// Load a `Library.xml` from disk, locating each material's texture in
    /// the `Textures` folder beside it.
    pub fn load_library_file(&mut self, path: &Path) -> usize {
        let Ok(xml) = std::fs::read_to_string(path) else {
            return 0;
        };
        // Textures may sit in sub-folders ("TEXTURES\Decals\..."); names are
        // unique enough to match on the file name alone.
        let textures: HashMap<String, PathBuf> = path
            .parent()
            .map(|d| WalkDir::new(d.join("Textures")))
            .into_iter()
            .flat_map(|w| w.into_iter().filter_map(Result::ok))
            .filter(|e| e.file_type().is_file())
            .map(|e| (e.file_name().to_string_lossy().to_ascii_lowercase(), e.path().to_path_buf()))
            .collect();
        let found = parse_library(&xml);
        let n = found.len();
        for (guid, mut info) in found {
            info.texture_path = info.decal_texture.as_ref().and_then(|t| textures.get(&t.to_ascii_lowercase()).cloned());
            self.insert(guid, info);
        }
        n
    }

    /// Load every `Library.xml` under a package's `MaterialLibs` folder.
    pub fn load_package(&mut self, root: &Path) -> usize {
        let libs = root.join("MaterialLibs");
        if !libs.is_dir() {
            return 0;
        }
        WalkDir::new(libs)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().eq_ignore_ascii_case("Library.xml"))
            .map(|e| self.load_library_file(e.path()))
            .sum()
    }

    /// Load the simulator's stock material libraries, if an MSFS 2020 install
    /// is present. MSFS 2024 streams its stock content, so only 2020's copy of
    /// the shared Asobo libraries is normally on disk.
    pub fn load_stock(&mut self) -> usize {
        let Some(root) = stock_package_root() else {
            return 0;
        };
        let mut n = 0;
        for vendor in ["OneStore", "Steam"] {
            let dir = root.join("Official").join(vendor);
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.filter_map(Result::ok) {
                let p = entry.path();
                if p.join("MaterialLibs").is_dir() {
                    n += self.load_package(&p);
                }
            }
        }
        n
    }
}

/// Every material in a `Library.xml`.
fn parse_library(xml: &str) -> Vec<(Guid, MaterialInfo)> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    doc.descendants()
        .filter(|n| n.has_tag_name("Material"))
        .filter_map(|node| {
            let guid = node.attribute("Guid").and_then(|g| g.parse::<Guid>().ok())?;
            let decal_texture = node
                .descendants()
                .filter(|t| t.has_tag_name("Texture"))
                .find(|t| t.attribute("Binding") == Some("MTL_BITMAP_DECAL0"))
                .and_then(|t| t.attribute("FileName"))
                .map(|f| f.rsplit(|c: char| c == '/' || c as u32 == 92).next().unwrap_or(f).to_string());
            Some((
                guid,
                MaterialInfo {
                    name: node.attribute("Name").unwrap_or_default().to_string(),
                    surface_type: node.attribute("SurfaceType").unwrap_or_default().to_string(),
                    decal_texture,
                    texture_path: None,
                },
            ))
        })
        .collect()
}

/// The MSFS 2020 install's official package folders (`Official/OneStore`,
/// `Official/Steam`), where the stock model and material libraries live.
pub fn stock_official_dirs() -> Vec<PathBuf> {
    let Some(root) = stock_package_root() else {
        return Vec::new();
    };
    ["OneStore", "Steam"]
        .iter()
        .map(|v| root.join("Official").join(v))
        .filter(|d| d.is_dir())
        .collect()
}

fn stock_package_root() -> Option<PathBuf> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    let roaming = std::env::var("APPDATA").unwrap_or_default();
    let candidates = [
        Path::new(&local).join("Packages/Microsoft.FlightSimulator_8wekyb3d8bbwe/LocalCache/UserCfg.opt"),
        Path::new(&roaming).join("Microsoft Flight Simulator/UserCfg.opt"),
    ];
    for cfg in candidates {
        let Ok(text) = std::fs::read_to_string(&cfg) else {
            continue;
        };
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("InstalledPackagesPath") {
                return Some(PathBuf::from(rest.trim().trim_matches('"')));
            }
        }
    }
    None
}

fn has_any(name: &str, words: &[&str]) -> bool {
    words.iter().any(|w| name.contains(w))
}

/// Ground materials that are markings or dirt layered over the pavement, not
/// the pavement itself.
const DECAL_WORDS: &[&str] = &[
    "number", "letter", "decal", "zebra", "chevron", "squares", "gp_frame", "grime", "dirt_turn", "runway_dirt", "tirebend", "arrow", "tire", "tyre", "skid", "hatch", "atlas", "stain", "oil", "crack", "seam",
    "grunge", "text", "logo", "sign",
];

/// What a ground polygon's material says about it: `Some(surface)` for
/// pavement, `None` when it is a decal or cannot be classified by name.
pub fn classify_ground(info: &MaterialInfo) -> GroundClass {
    let name = info.name.to_ascii_lowercase();
    // iniBuilds "INI_Dirt_*" materials are grime laid over pavement, not dirt ground.
    if has_any(&name, DECAL_WORDS) || name.starts_with("ini_dirt_") {
        return GroundClass::Decal;
    }
    // Hard surfaces are checked first: Asobo's "CEMENTDIRT01" and
    // "ConcreteDirt01" are grey cement (tagged Asobo_Concrete, cement
    // textures) and "AsphaltFloor_Dirt01" is asphalt; the "dirt" means worn.
    // Read as dirt, Dubai's stands and aprons came out as sand.
    let by_name = if has_any(&name, &["concrete", "cement", "beton"]) {
        Some(Surface::Concrete)
    } else if has_any(&name, &["asphalt", "tarmac", "bitum", "macadam"]) {
        Some(Surface::Asphalt)
    } else if has_any(&name, &["dirt", "sand", "soil", "clay", "mud"]) {
        Some(Surface::Dirt)
    } else if has_any(&name, &["grass", "turf"]) {
        Some(Surface::Grass)
    } else if name.contains("gravel") {
        Some(Surface::Gravel)
    } else {
        None
    };
    if let Some(s) = by_name {
        return GroundClass::Pavement(s);
    }
    match info.surface_type.to_ascii_uppercase().as_str() {
        "CONCRETE" | "CEMENT" => GroundClass::Pavement(Surface::Concrete),
        "ASPHALT" | "BITUMINUS" | "BITUMINOUS" | "TARMAC" | "MACADAM" => GroundClass::Pavement(Surface::Asphalt),
        "GRASS" | "SHORT_GRASS" | "LONG_GRASS" | "GRASS_BUMPY" | "HARD_TURF" => GroundClass::Pavement(Surface::Grass),
        "DIRT" | "SAND" | "CLAY" => GroundClass::Pavement(Surface::Dirt),
        "GRAVEL" => GroundClass::Pavement(Surface::Gravel),
        "PAINT" => GroundClass::Decal,
        _ => GroundClass::Unknown,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroundClass {
    Pavement(Surface),
    Decal,
    Unknown,
}

/// Asphalt or concrete from a tint colour, for materials no library names.
///
/// MSFS multiplies the texture by the tint, so a light tint is almost always a
/// concrete pad and a dark one asphalt. A heuristic, reported as such.
pub fn surface_from_tint(rgba: [u8; 4]) -> Option<Surface> {
    if rgba[3] == 0 {
        return None;
    }
    let lum = 0.299 * rgba[0] as f32 + 0.587 * rgba[1] as f32 + 0.114 * rgba[2] as f32;
    Some(if lum >= 90.0 {
        Surface::Concrete
    } else {
        Surface::Asphalt
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineColour {
    Yellow,
    White,
    Red,
    Unknown,
}

/// What a painted line's material name says about its appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineLook {
    pub colour: LineColour,
    pub dashed: bool,
    pub hold_short: bool,
    /// Painted with a black border (iniBuilds' taxiway lines).
    pub bordered: bool,
    /// A taxiway edge line rather than a centreline.
    pub edge: bool,
    /// Outlines, seams and faded lines: texture detail X-Plane draws itself.
    pub skip: bool,
}

pub fn classify_line(info: &MaterialInfo) -> LineLook {
    classify_line_name(&info.name)
}

/// [`classify_line`] for a bare material name.
pub fn classify_line_name(name: &str) -> LineLook {
    let name = name.to_ascii_lowercase();
    // iniBuilds' taxiway lines "INI_CenterLine_Black" and "INI_Edge_Line_Black"
    // are yellow lines with a black border (their textures show it), not
    // black outlines; they carry every centreline and edge line at Dubai.
    let bordered = name.contains("black") && has_any(&name, &["centerline", "centreline", "edge_line", "edgeline"]);
    let white = name.contains("white");
    let yellow = bordered || name.contains("yellow") || name.contains("_wy") || name.contains("wy_");
    let red = name.contains("red");
    let colour = if yellow {
        LineColour::Yellow
    } else if white {
        LineColour::White
    } else if red {
        LineColour::Red
    } else {
        LineColour::Unknown
    };
    LineLook {
        colour,
        // "Dasjed" is a real typo in a shipping iniBuilds material name.
        dashed: has_any(&name, &["dash", "dasjed", "broken"]),
        hold_short: name.contains("hold"),
        bordered,
        edge: name.contains("edge"),
        skip: !bordered && has_any(&name, &["black", "seam", "faded", "shadow", "grunge", "crack"]),
    }
}

/// How an airport's materials were resolved.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResolveStats {
    pub aprons_named: usize,
    pub aprons_from_tint: usize,
    pub aprons_defaulted: usize,
    pub lines_named: usize,
    pub runways_named: usize,
    /// Aprons whose material is not on this PC, textured with a stock
    /// stand-in of the same class.
    pub aprons_stand_in_texture: usize,
}

/// A stock material to texture pavement whose own material is not on this PC
/// (MSFS 2024 streams its stock ground materials). Among the simulator's plain
/// asphalts and concretes, the one whose texture is nearest a typical airport
/// shade: dark grey taxiway asphalt, mid-grey concrete. Stock names repeat
/// across libraries with textures from near black to near white, so the
/// choice is by measured brightness, not by name alone.
fn stand_in_material(catalog: &MaterialCatalog, surface: Surface) -> Option<(&MaterialInfo, u8)> {
    let (names, target): (&[&str], i32) = match surface {
        Surface::Asphalt => (&["Asphalt_Generic05", "Asphalt_Generic06", "Asphalt_Generic03", "Asphalt_Generic02", "Taxi_Asphalt"], 85),
        Surface::Concrete => (&["ConcreteDirt01", "Aso_Concrete", "Concrete11", "CEMENTDIRT01", "Tile_Concrete01"], 125),
        _ => return None,
    };
    names
        .iter()
        .flat_map(|n| catalog.find_by_name(n))
        .filter_map(|m| brightness(m).map(|b| (m, b)))
        .min_by_key(|(_, b)| (*b as i32 - target).abs())
}

/// Give every apron a surface and every painted line a material name.
pub fn resolve_airport(ap: &mut Airport, catalog: &MaterialCatalog) -> ResolveStats {
    let mut stats = ResolveStats::default();
    // Texture brightness is read once per material.
    let mut bright: HashMap<Guid, Option<u8>> = HashMap::new();
    let mut brightness_of = |guid: Guid, info: &MaterialInfo| *bright.entry(guid).or_insert_with(|| brightness(info));
    let stand_ins: HashMap<Surface, (&MaterialInfo, u8)> = [Surface::Asphalt, Surface::Concrete]
        .into_iter()
        .filter_map(|s| stand_in_material(catalog, s).map(|m| (s, m)))
        .collect();
    for a in &mut ap.aprons {
        let Some(guid) = a.material_guid else { continue };
        match catalog.get(&guid) {
            Some(info) => {
                a.material_name = Some(info.name.clone());
                a.decal_texture = info.decal_texture.clone();
                match classify_ground(info) {
                    GroundClass::Pavement(s) => {
                        a.surface = s;
                        a.brightness = brightness_of(guid, info);
                    }
                    GroundClass::Decal => a.draw = false,
                    GroundClass::Unknown => a.surface = surface_from_tint(a.tint).unwrap_or(Surface::Asphalt),
                }
                stats.aprons_named += 1;
            }
            None => match surface_from_tint(a.tint) {
                Some(s) => {
                    a.surface = s;
                    stats.aprons_from_tint += 1;
                    if let Some((m, b)) = stand_ins.get(&s) {
                        a.decal_texture = m.decal_texture.clone();
                        a.brightness = Some(*b);
                        stats.aprons_stand_in_texture += 1;
                    }
                }
                None => {
                    a.surface = Surface::Asphalt;
                    stats.aprons_defaulted += 1;
                }
            },
        }
    }
    for l in &mut ap.painted_lines {
        if let Some(info) = l.material_guid.and_then(|g| catalog.get(&g)) {
            l.material_name = Some(info.name.clone());
            stats.lines_named += 1;
        }
    }
    for p in &mut ap.taxi_paths {
        let Some(guid) = p.material_guid else { continue };
        if let Some(info) = catalog.get(&guid) {
            p.material_name = Some(info.name.clone());
            if let GroundClass::Pavement(s) = classify_ground(info) {
                p.surface = s;
                p.brightness = brightness_of(guid, info);
            }
        }
    }
    for r in &mut ap.runways {
        let Some(guid) = r.material_guid else { continue };
        if let Some(info) = catalog.get(&guid) {
            r.material_name = Some(info.name.clone());
            stats.runways_named += 1;
            if let GroundClass::Pavement(s) = classify_ground(info) {
                r.surface = s;
                r.brightness = brightness_of(guid, info);
            }
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, surface: &str) -> MaterialInfo {
        MaterialInfo {
            name: name.into(),
            surface_type: surface.into(),
            decal_texture: None,
            texture_path: None,
        }
    }

    #[test]
    fn brightness_comes_from_the_texture_next_to_the_library() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Textures")).unwrap();
        image::RgbaImage::from_pixel(8, 8, image::Rgba([100, 100, 100, 255]))
            .save(dir.path().join("Textures").join("DARK.PNG"))
            .unwrap();
        std::fs::write(
            dir.path().join("Library.xml"),
            r#"<Library><Material Name="INI_Asphalt_9" Guid="{BA5CE1B0-FEE5-42C1-8AF1-75FC656D3226}" SurfaceType="CEMENT">
                <TextureList><Texture FileName="TEXTURES\dark.png" Binding="MTL_BITMAP_DECAL0"/></TextureList></Material></Library>"#,
        )
        .unwrap();
        let mut cat = MaterialCatalog::default();
        assert_eq!(cat.load_library_file(&dir.path().join("Library.xml")), 1);
        let g: Guid = "{BA5CE1B0-FEE5-42C1-8AF1-75FC656D3226}".parse().unwrap();
        let m = cat.get(&g).unwrap();
        assert!(m.texture_path.is_some(), "found despite the case difference");
        assert_eq!(brightness(m), Some(100));
        assert_eq!(brightness(&info("x", "y")), None);
    }

    const LIB: &str = r#"<Library Version="1.1.0">
        <Material Version="1.5.0" Name="INI_Letter_B_Thin" Guid="{EE189856-C5C9-4136-9679-376F12F4BFD7}" SurfaceType="PAINT"/>
        <Material Name="INI_Asphalt_1" Guid="{BA5CE1B0-FEE5-42C1-8AF1-75FC656D3225}" SurfaceType="CONCRETE"><TagList/></Material>
        <Material Name="no guid"/>
    </Library>"#;

    #[test]
    fn loads_a_library_by_guid() {
        let mut cat = MaterialCatalog::default();
        assert_eq!(cat.load_library_text(LIB), 2);
        let g: Guid = "{BA5CE1B0-FEE5-42C1-8AF1-75FC656D3225}".parse().unwrap();
        assert_eq!(cat.get(&g).unwrap().name, "INI_Asphalt_1");
        assert_eq!(cat.load_library_text("not xml"), 0);
    }

    #[test]
    fn unknown_ground_materials_get_a_stock_stand_in_texture() {
        // Two stock asphalts and two concretes, at different brightnesses:
        // the stand-in is the one nearest a typical airport shade.
        let dir = tempfile::tempdir().unwrap();
        let tex = dir.path().join("Textures");
        std::fs::create_dir_all(&tex).unwrap();
        let mut xml = String::from("<Library>");
        for (i, (name, level)) in [("Asphalt_Generic03", 40u8), ("Asphalt_Generic05", 90), ("ConcreteDirt01", 200), ("Aso_Concrete", 120)]
            .iter()
            .enumerate()
        {
            image::RgbaImage::from_pixel(4, 4, image::Rgba([*level, *level, *level, 255]))
                .save(tex.join(format!("{name}.png")))
                .unwrap();
            xml.push_str(&format!(
                r#"<Material Name="{name}" Guid="{{{i}{i}{i}{i}{i}{i}{i}{i}-1111-1111-1111-111111111111}}" SurfaceType="CEMENT">
                   <TextureList><Texture FileName="TEXTURES\{name}.png" Binding="MTL_BITMAP_DECAL0"/></TextureList></Material>"#
            ));
        }
        xml.push_str("</Library>");
        std::fs::write(dir.path().join("Library.xml"), xml).unwrap();
        let mut cat = MaterialCatalog::default();
        assert_eq!(cat.load_library_file(&dir.path().join("Library.xml")), 4);
        let streamed: Guid = "{8E26DDCB-02D6-4436-972E-3D8921C4EFB4}".parse().unwrap();
        let apron = |tint| Apron {
            draw: true,
            material_guid: Some(streamed),
            tint,
            ..Default::default()
        };
        let mut ap = Airport {
            aprons: vec![apron([0, 0, 1, 255]), apron([114, 121, 123, 255]), apron([0, 0, 0, 0])],
            ..Default::default()
        };
        let st = resolve_airport(&mut ap, &cat);
        assert_eq!((st.aprons_from_tint, st.aprons_stand_in_texture, st.aprons_defaulted), (2, 2, 1));
        assert_eq!(ap.aprons[0].surface, Surface::Asphalt);
        assert_eq!(ap.aprons[0].decal_texture.as_deref(), Some("Asphalt_Generic05.png"), "90 is nearer 85 than 40");
        assert_eq!(ap.aprons[0].brightness, Some(90));
        assert_eq!(ap.aprons[1].surface, Surface::Concrete);
        assert_eq!(ap.aprons[1].decal_texture.as_deref(), Some("Aso_Concrete.png"), "120 is nearer 125 than 200");
        assert!(ap.aprons[2].decal_texture.is_none(), "no tint, nothing to go on");
    }

    #[test]
    fn names_beat_the_physics_surface_type() {
        assert_eq!(
            classify_ground(&info("INI_Asphalt_1", "CONCRETE")),
            GroundClass::Pavement(Surface::Asphalt)
        );
        // Worn pavement, not dirt ground: Asobo tags these concrete and asphalt.
        assert_eq!(
            classify_ground(&info("CEMENTDIRT01", "CEMENT")),
            GroundClass::Pavement(Surface::Concrete)
        );
        assert_eq!(
            classify_ground(&info("ConcreteDirt01", "CEMENT")),
            GroundClass::Pavement(Surface::Concrete)
        );
        assert_eq!(
            classify_ground(&info("AsphaltFloor_Dirt01", "CEMENT")),
            GroundClass::Pavement(Surface::Asphalt)
        );
        assert_eq!(classify_ground(&info("MudSand", "UNDEFINED")), GroundClass::Pavement(Surface::Dirt));
        assert_eq!(classify_ground(&info("Groud_sand", "DIRT")), GroundClass::Pavement(Surface::Dirt));
        assert_eq!(
            classify_ground(&info("PaintRough01", "CEMENT")),
            GroundClass::Pavement(Surface::Concrete)
        );
        assert_eq!(classify_ground(&info("INI_Number_3", "PAINT")), GroundClass::Decal);
        assert_eq!(classify_ground(&info("INI_Dirt_TireBend", "PAINT")), GroundClass::Decal);
        assert_eq!(classify_ground(&info("INI_Dirt_Turn90", "ASPHALT")), GroundClass::Decal);
        assert_eq!(classify_ground(&info("INI_Runway_Dirt", "ASPHALT")), GroundClass::Decal);
        assert_eq!(classify_ground(&info("INI_Asphalt_1_gp_frame", "ASPHALT")), GroundClass::Decal);
        assert_eq!(classify_ground(&info("INI_Zebra", "CONCRETE")), GroundClass::Decal);
        assert_eq!(classify_ground(&info("Mystery", "UNDEFINED")), GroundClass::Unknown);
    }

    #[test]
    fn tint_decides_between_asphalt_and_concrete() {
        assert_eq!(surface_from_tint([0x69, 0x6E, 0x72, 0xFF]), Some(Surface::Concrete));
        assert_eq!(surface_from_tint([0x42, 0x45, 0x47, 0xFF]), Some(Surface::Asphalt));
        assert_eq!(surface_from_tint([0, 0, 0, 0]), None);
    }

    #[test]
    fn line_names_give_colour_and_dash() {
        let l = classify_line(&info("INI_Lines_Yellow_Dashed", "PAINT"));
        assert_eq!((l.colour, l.dashed, l.skip), (LineColour::Yellow, true, false));
        let l = classify_line(&info("INI_Lines_Dashed_White", "UNDEFINED"));
        assert_eq!((l.colour, l.dashed), (LineColour::White, true));
        let l = classify_line(&info("INI_CenterLine_Black_Dasjed", "CONCRETE"));
        assert_eq!((l.colour, l.dashed, l.bordered, l.skip), (LineColour::Yellow, true, true, false));
        let l = classify_line(&info("INI_CenterLine_Black", "CEMENT"));
        assert_eq!((l.colour, l.dashed, l.bordered, l.edge, l.skip), (LineColour::Yellow, false, true, false, false));
        let l = classify_line(&info("INI_Edge_Line_Black", "CEMENT"));
        assert_eq!((l.colour, l.bordered, l.edge, l.skip), (LineColour::Yellow, true, true, false));
        assert!(classify_line(&info("Asphalt05_BLack", "ASPHALT")).skip, "other black lines stay out");
        let red = classify_line(&info("INI_Red_Lines", "PAINT"));
        assert_eq!((red.colour, red.skip), (LineColour::Red, false), "X-Plane has red lines");
        assert!(!classify_line(&info("INI_Red_White", "PAINT")).skip);
        assert!(classify_line(&info("INI_Seam", "CONCRETE")).skip);
        assert_eq!(classify_line(&info("INI_Lines_WY", "PAINT")).colour, LineColour::Yellow);
    }
}
