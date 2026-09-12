//! Raw record structures: a faithful, un-interpreted view of what a BGL holds.
//!
//! Everything here stays close to the file format (codes are still numbers,
//! coordinates are already decoded to degrees/metres because that is lossless).
//! Interpretation into the sim-neutral model happens in `crate::model`.

/// Which record layout family a file (or a single record) uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    Fs9,
    Fsx,
    P3dV4,
    P3dV5,
    Msfs2020,
    Msfs2024,
    #[default]
    Unknown,
}

impl Variant {
    /// True for the two MSFS generations, which share most layout extensions.
    pub fn is_msfs(self) -> bool {
        matches!(self, Variant::Msfs2020 | Variant::Msfs2024)
    }

    pub fn label(self) -> &'static str {
        match self {
            Variant::Fs9 => "FS9",
            Variant::Fsx => "FSX",
            Variant::P3dV4 => "P3D v4",
            Variant::P3dV5 => "P3D v5",
            Variant::Msfs2020 => "MSFS 2020",
            Variant::Msfs2024 => "MSFS 2024",
            Variant::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RawAirport {
    pub ident: String,
    pub region: String,
    pub lat: f64,
    pub lon: f64,
    pub alt_m: f64,
    pub tower_lat: f64,
    pub tower_lon: f64,
    pub tower_alt_m: f64,
    pub magvar: f32,
    pub fuel_flags: u32,
    pub closed: bool,
    pub name: String,
    pub has_tower_obj: bool,
    pub runways: Vec<RawRunway>,
    pub coms: Vec<RawCom>,
    pub starts: Vec<RawStart>,
    pub helipads: Vec<RawHelipad>,
    pub taxi_points: Vec<RawTaxiPoint>,
    pub taxi_names: Vec<String>,
    pub taxi_paths: Vec<RawTaxiPath>,
    pub parkings: Vec<RawParking>,
    pub aprons: Vec<RawApron>,
    pub apron_edge_lights: Vec<Vec<(f64, f64)>>,
    pub painted_lines: Vec<RawPaintedLine>,
    pub signs: Vec<RawSign>,
    /// `(parking number, parking name code)` pairs from jetway records.
    pub jetways: Vec<(u16, u16)>,
    pub delete_airport: bool,
    pub variant: Variant,
    pub warnings: Vec<String>,
    /// `(record id, byte size)` of records we chose not to interpret.
    pub unknown_records: Vec<(u16, usize)>,
}

#[derive(Debug, Clone, Default)]
pub struct RawRunway {
    pub surface: u8,
    pub lat: f64,
    pub lon: f64,
    pub alt_m: f64,
    pub length_m: f32,
    pub width_m: f32,
    pub heading_true: f32,
    pub pattern_alt_m: f32,
    pub marking_flags: u32,
    pub light_flags: u8,
    pub pattern_flags: u8,
    pub primary: RawRunwayEnd,
    pub secondary: RawRunwayEnd,
}

#[derive(Debug, Clone, Default)]
pub struct RawRunwayEnd {
    pub number: u8,
    pub designator: u8,
    pub ils_ident: String,
    pub offset_threshold_m: f32,
    pub blast_pad_m: f32,
    pub overrun_m: f32,
    pub vasi_left: Option<RawVasi>,
    pub vasi_right: Option<RawVasi>,
    pub approach_lights: Option<RawApproachLights>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RawVasi {
    pub kind: u16,
    pub bias_x: f32,
    pub bias_z: f32,
    pub spacing: f32,
    pub pitch: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RawApproachLights {
    pub system: u8,
    pub end_lights: bool,
    pub reil: bool,
    pub touchdown: bool,
    pub strobes: u8,
}

#[derive(Debug, Clone, Default)]
pub struct RawCom {
    pub kind: u16,
    pub freq_khz: u32,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RawStart {
    pub number: u8,
    pub designator: u8,
    pub kind: u8,
    pub lat: f64,
    pub lon: f64,
    pub alt_m: f64,
    pub heading: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RawHelipad {
    pub surface: u8,
    pub kind: u8,
    pub transparent: bool,
    pub closed: bool,
    pub lat: f64,
    pub lon: f64,
    pub alt_m: f64,
    pub length_m: f32,
    pub width_m: f32,
    pub heading: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RawTaxiPoint {
    pub kind: u8,
    pub orientation: u8,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Default)]
pub struct RawTaxiPath {
    pub start: u16,
    pub end: u16,
    pub runway_number: u8,
    pub runway_designator: u8,
    pub kind: u8,
    pub draw_surface: bool,
    pub draw_detail: bool,
    pub name_index: u8,
    pub centerline: bool,
    pub centerline_lit: bool,
    pub left_edge: u8,
    pub left_edge_lit: bool,
    pub right_edge: u8,
    pub right_edge_lit: bool,
    pub surface: u8,
    pub width_m: f32,
}

#[derive(Debug, Clone, Default)]
pub struct RawParking {
    pub name: u8,
    pub push_back: u8,
    pub kind: u8,
    pub number: u16,
    pub radius_m: f32,
    pub heading: f32,
    pub lat: f64,
    pub lon: f64,
    pub airlines: Vec<String>,
    pub suffix: u8,
    pub tee_offsets: [f32; 4],
}

#[derive(Debug, Clone, Default)]
pub struct RawApron {
    pub surface: u8,
    pub draw_surface: bool,
    pub draw_detail: bool,
    /// `(lat, lon)` boundary vertices.
    pub vertices: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Default)]
pub struct RawPaintedLine {
    pub kind: u16,
    pub vertices: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Default)]
pub struct RawSign {
    pub lat: f64,
    pub lon: f64,
    pub heading: f32,
    pub size: u8,
    pub justification: u8,
    pub label: String,
}
