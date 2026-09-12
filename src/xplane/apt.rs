//! The apt.dat 1200 document model.
//!
//! X-Plane's airport file is a flat list of numbered rows whose meaning depends
//! on position, which is easy to get subtly wrong. Modelling each row as a typed
//! struct means the writer cannot transpose two columns, and the validator has
//! something concrete to check against.
//!
//! Row codes used here come from the Laminar apt.dat 1200 specification.

/// Row codes, named as the specification names them.
pub mod row {
    pub const LAND_AIRPORT: u16 = 1;
    pub const SEAPLANE_BASE: u16 = 16;
    pub const HELIPORT: u16 = 17;
    pub const TOWER: u16 = 14;
    pub const BEACON: u16 = 18;
    pub const WINDSOCK: u16 = 19;
    pub const SIGN: u16 = 20;
    pub const LIGHT_OBJECT: u16 = 21;
    pub const RUNWAY: u16 = 100;
    pub const WATER_RUNWAY: u16 = 101;
    pub const HELIPAD: u16 = 102;
    pub const PAVEMENT: u16 = 110;
    pub const NODE: u16 = 111;
    pub const NODE_BEZIER: u16 = 112;
    pub const NODE_CLOSE: u16 = 113;
    pub const NODE_CLOSE_BEZIER: u16 = 114;
    pub const NODE_END: u16 = 115;
    pub const NODE_END_BEZIER: u16 = 116;
    pub const LINEAR_FEATURE: u16 = 120;
    pub const BOUNDARY: u16 = 130;
    pub const FREQ_AWOS: u16 = 1050;
    pub const FREQ_CTAF: u16 = 1051;
    pub const FREQ_CLEARANCE: u16 = 1052;
    pub const FREQ_GROUND: u16 = 1053;
    pub const FREQ_TOWER: u16 = 1054;
    pub const FREQ_APPROACH: u16 = 1055;
    pub const FREQ_DEPARTURE: u16 = 1056;
    pub const TAXI_NETWORK: u16 = 1200;
    pub const TAXI_NODE: u16 = 1201;
    pub const TAXI_EDGE: u16 = 1202;
    pub const TAXI_SHAPE: u16 = 1203;
    pub const TAXI_ACTIVE_ZONE: u16 = 1204;
    pub const TAXI_TRUCK_EDGE: u16 = 1206;
    pub const RAMP_START: u16 = 1300;
    pub const RAMP_START_META: u16 = 1301;
    pub const METADATA: u16 = 1302;
    pub const TRUCK_PARKING: u16 = 1400;
    pub const TRUCK_DESTINATION: u16 = 1401;
    pub const JETWAY: u16 = 1500;
    pub const FILE_END: u16 = 99;
}

/// Surface codes. The 1200 specification added shaded asphalt and concrete
/// variants (20-38 and 50-57); the classic 1-15 codes remain valid.
pub mod surface {
    pub const ASPHALT: u8 = 1;
    pub const CONCRETE: u8 = 2;
    pub const GRASS: u8 = 3;
    pub const DIRT: u8 = 4;
    pub const GRAVEL: u8 = 5;
    pub const DRY_LAKEBED: u8 = 12;
    pub const WATER: u8 = 13;
    pub const SNOW_ICE: u8 = 14;
    pub const TRANSPARENT: u8 = 15;
}

/// One end of a land runway.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunwayEnd {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub displaced_m: f32,
    pub blast_pad_m: f32,
    /// 0 none, 1 visual, 2 non-precision, 3 precision, 4/5 UK, 6/7 EASA.
    pub markings: u8,
    /// 0 none through 12 RAIL.
    pub approach_lights: u8,
    pub touchdown_lights: bool,
    /// 0 none, 1 omnidirectional, 2 unidirectional.
    pub reil: u8,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Runway {
    pub width_m: f32,
    pub surface: u8,
    pub shoulder: u8,
    pub smoothness: f32,
    pub centre_lights: bool,
    /// 0 none, 1 low, 2 medium, 3 high.
    pub edge_lights: u8,
    pub distance_signs: bool,
    pub ends: [RunwayEnd; 2],
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WaterRunway {
    pub width_m: f32,
    pub buoys: bool,
    pub ends: [(String, f64, f64); 2],
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Helipad {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub heading: f32,
    pub length_m: f32,
    pub width_m: f32,
    pub surface: u8,
    pub markings: u8,
    pub shoulder: u8,
    pub smoothness: f32,
    pub edge_lights: u8,
}

/// A node inside a pavement ring or a linear feature.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Node {
    pub lat: f64,
    pub lon: f64,
    /// Bezier control point, mirrored by X-Plane about the node.
    pub control: Option<(f64, f64)>,
    /// Painted line code, 0 for none.
    pub line: u8,
    /// Light string code, 0 for none.
    pub light: u8,
}

impl Node {
    pub fn at(lat: f64, lon: f64) -> Self {
        Node {
            lat,
            lon,
            ..Default::default()
        }
    }
}

/// A filled polygon: ring 0 is the outline, the rest are holes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pavement {
    pub surface: u8,
    pub smoothness: f32,
    /// Texture orientation in degrees true.
    pub heading: f32,
    pub name: String,
    pub rings: Vec<Vec<Node>>,
}

/// A painted line or light string, open or closed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinearFeature {
    pub name: String,
    pub nodes: Vec<Node>,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tower {
    pub lat: f64,
    pub lon: f64,
    pub height_ft: f32,
    pub draw: bool,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Beacon {
    pub lat: f64,
    pub lon: f64,
    /// 0 none, 1 airport, 2 seaport, 3 heliport, 4 military.
    pub kind: u8,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Windsock {
    pub lat: f64,
    pub lon: f64,
    pub lit: bool,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sign {
    pub lat: f64,
    pub lon: f64,
    pub heading: f32,
    /// 1 small through 5, where 4 and 5 are distance-remaining boards.
    pub size: u8,
    pub text: String,
}

/// A VASI, PAPI or wig-wag unit.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LightObject {
    pub lat: f64,
    pub lon: f64,
    /// 1 VASI, 2 PAPI left, 3 PAPI right, 4 space shuttle, 5 tri-colour,
    /// 6 wig-wag, 7 APAPI left, 8 APAPI right.
    pub kind: u8,
    pub heading: f32,
    pub angle: f32,
    pub runway: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frequency {
    pub code: u16,
    /// Kilohertz, six digits in the file.
    pub khz: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NetNode {
    pub lat: f64,
    pub lon: f64,
    /// `dest`, `init`, `both` or `junc`.
    pub usage: String,
    pub id: usize,
    pub name: String,
}

/// A runway that an edge is a hot zone for, and in which phase of flight.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActiveZone {
    /// `departure`, `arrival` or `ils`.
    pub phase: String,
    pub runways: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NetEdge {
    pub from: usize,
    pub to: usize,
    pub oneway: bool,
    /// `runway` or `taxiway`, the latter optionally suffixed `_A` to `_F`.
    pub kind: String,
    pub name: String,
    /// Intermediate shape points.
    pub shape: Vec<(f64, f64)>,
    pub active_zones: Vec<ActiveZone>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TruckEdge {
    pub from: usize,
    pub to: usize,
    pub oneway: bool,
    pub name: String,
    pub shape: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Network {
    pub name: String,
    pub nodes: Vec<NetNode>,
    pub edges: Vec<NetEdge>,
    pub truck_edges: Vec<TruckEdge>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RampStart {
    pub lat: f64,
    pub lon: f64,
    pub heading: f32,
    /// `gate`, `hangar`, `misc` or `tie_down`.
    pub location: String,
    /// `heavy`, `jets`, `turboprops`, `props`, `helos`, `fighters`, or `all`.
    pub aircraft: Vec<String>,
    pub name: String,
    /// ICAO width code A to F.
    pub width: char,
    /// `none`, `general_aviation`, `airline`, `cargo` or `military`.
    pub operations: String,
    pub airlines: Vec<String>,
}

/// An X-Plane native animated jetway attached to a stand.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Jetway {
    pub lat: f64,
    pub lon: f64,
    pub install_heading: f32,
    /// 0 glass, 1 solid light, 2 solid dark, 3 scaffold.
    pub style: u8,
    /// 0 small, 1 medium, 2 large.
    pub size: u8,
    pub parked_tunnel_heading: f32,
    pub parked_tunnel_length: f32,
    pub parked_cab_heading: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TruckParking {
    pub lat: f64,
    pub lon: f64,
    pub heading: f32,
    /// `baggage_loader`, `baggage_train`, `crew_car`, `fuel_jet`, `pushback` and so on.
    pub kind: String,
    pub cars: u8,
    pub name: String,
}

/// One airport in the file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AptAirport {
    /// 1 land, 16 seaplane base, 17 heliport.
    pub kind: u16,
    pub elevation_ft: i32,
    pub icao: String,
    pub name: String,
    pub metadata: Vec<(String, String)>,
    pub runways: Vec<Runway>,
    pub water_runways: Vec<WaterRunway>,
    pub helipads: Vec<Helipad>,
    pub pavements: Vec<Pavement>,
    /// Airport boundary ring (row 130), empty for none. X-Plane flattens the
    /// terrain inside it when the `flatten` metadata key is 1.
    pub boundary: Vec<Node>,
    pub lines: Vec<LinearFeature>,
    pub tower: Option<Tower>,
    pub beacons: Vec<Beacon>,
    pub windsocks: Vec<Windsock>,
    pub signs: Vec<Sign>,
    pub light_objects: Vec<LightObject>,
    pub frequencies: Vec<Frequency>,
    pub network: Option<Network>,
    pub ramp_starts: Vec<RampStart>,
    pub jetways: Vec<Jetway>,
    pub truck_parking: Vec<TruckParking>,
}

/// A complete apt.dat document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Apt {
    pub airports: Vec<AptAirport>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_helper_sets_no_line_or_light() {
        let n = Node::at(25.25, 55.36);
        assert_eq!(n.line, 0);
        assert_eq!(n.light, 0);
        assert!(n.control.is_none());
    }

    #[test]
    fn row_codes_match_the_specification() {
        // Spot-check the codes that are easiest to transpose.
        assert_eq!(row::RUNWAY, 100);
        assert_eq!(row::WATER_RUNWAY, 101);
        assert_eq!(row::HELIPAD, 102);
        assert_eq!(row::PAVEMENT, 110);
        assert_eq!(row::LINEAR_FEATURE, 120);
        assert_eq!(row::TAXI_NODE, 1201);
        assert_eq!(row::TAXI_EDGE, 1202);
        assert_eq!(row::RAMP_START, 1300);
        assert_eq!(row::METADATA, 1302);
        assert_eq!(row::JETWAY, 1500);
        assert_eq!(row::FILE_END, 99);
    }
}
