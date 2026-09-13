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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialInfo {
    pub name: String,
    pub surface_type: String,
    /// File name of the decal texture (binding MTL_BITMAP_DECAL0), if any.
    pub decal_texture: Option<String>,
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

    /// Parse one `Library.xml`. Returns how many materials it defined.
    pub fn load_library_text(&mut self, xml: &str) -> usize {
        let Ok(doc) = roxmltree::Document::parse(xml) else {
            return 0;
        };
        let mut n = 0;
        for node in doc.descendants().filter(|n| n.has_tag_name("Material")) {
            let Some(guid) = node.attribute("Guid").and_then(|g| g.parse::<Guid>().ok()) else {
                continue;
            };
            let decal_texture = node
                .descendants()
                .filter(|t| t.has_tag_name("Texture"))
                .find(|t| t.attribute("Binding") == Some("MTL_BITMAP_DECAL0"))
                .and_then(|t| t.attribute("FileName"))
                .map(|f| f.rsplit(|c: char| c == '/' || c as u32 == 92).next().unwrap_or(f).to_string());
            self.insert(
                guid,
                MaterialInfo {
                    name: node.attribute("Name").unwrap_or_default().to_string(),
                    surface_type: node.attribute("SurfaceType").unwrap_or_default().to_string(),
                    decal_texture,
                },
            );
            n += 1;
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
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .map(|text| self.load_library_text(&text))
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
    // Soft ground is checked first: Asobo's "CEMENTDIRT01" is a dirt ground
    // cover used for infields, not a concrete pad.
    let by_name = if has_any(&name, &["dirt", "sand", "soil", "clay", "mud"]) {
        Some(Surface::Dirt)
    } else if has_any(&name, &["grass", "turf"]) {
        Some(Surface::Grass)
    } else if has_any(&name, &["concrete", "cement", "beton"]) {
        Some(Surface::Concrete)
    } else if has_any(&name, &["asphalt", "tarmac", "bitum", "macadam"]) {
        Some(Surface::Asphalt)
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
    /// Outlines, seams and faded lines: texture detail X-Plane draws itself.
    pub skip: bool,
}

pub fn classify_line(info: &MaterialInfo) -> LineLook {
    classify_line_name(&info.name)
}

/// [`classify_line`] for a bare material name.
pub fn classify_line_name(name: &str) -> LineLook {
    let name = name.to_ascii_lowercase();
    let white = name.contains("white");
    let yellow = name.contains("yellow") || name.contains("_wy") || name.contains("wy_");
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
        skip: has_any(&name, &["black", "seam", "faded", "shadow", "grunge", "crack"]),
    }
}

/// How an airport's materials were resolved.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResolveStats {
    pub aprons_named: usize,
    pub aprons_from_tint: usize,
    pub aprons_defaulted: usize,
    pub lines_named: usize,
}

/// Give every apron a surface and every painted line a material name.
pub fn resolve_airport(ap: &mut Airport, catalog: &MaterialCatalog) -> ResolveStats {
    let mut stats = ResolveStats::default();
    for a in &mut ap.aprons {
        let Some(guid) = a.material_guid else { continue };
        match catalog.get(&guid) {
            Some(info) => {
                a.material_name = Some(info.name.clone());
                a.decal_texture = info.decal_texture.clone();
                match classify_ground(info) {
                    GroundClass::Pavement(s) => a.surface = s,
                    GroundClass::Decal => a.draw = false,
                    GroundClass::Unknown => a.surface = surface_from_tint(a.tint).unwrap_or(Surface::Asphalt),
                }
                stats.aprons_named += 1;
            }
            None => match surface_from_tint(a.tint) {
                Some(s) => {
                    a.surface = s;
                    stats.aprons_from_tint += 1;
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
        }
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
    fn names_beat_the_physics_surface_type() {
        assert_eq!(
            classify_ground(&info("INI_Asphalt_1", "CONCRETE")),
            GroundClass::Pavement(Surface::Asphalt)
        );
        assert_eq!(
            classify_ground(&info("CEMENTDIRT01", "CEMENT")),
            GroundClass::Pavement(Surface::Dirt)
        );
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
        assert!(classify_line(&info("INI_CenterLine_Black_Dasjed", "CONCRETE")).skip);
        let red = classify_line(&info("INI_Red_Lines", "PAINT"));
        assert_eq!((red.colour, red.skip), (LineColour::Red, false), "X-Plane has red lines");
        assert!(!classify_line(&info("INI_Red_White", "PAINT")).skip);
        assert!(classify_line(&info("INI_Seam", "CONCRETE")).skip);
        assert_eq!(classify_line(&info("INI_Lines_WY", "PAINT")).colour, LineColour::Yellow);
    }
}
