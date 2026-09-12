//! The simulator-neutral airport model.
//!
//! Everything upstream (BGL records, SDK XML) is converted into these types, and
//! everything downstream (apt.dat rows, previews, reports) is generated from
//! them. Keeping a neutral layer in the middle means a new input format needs
//! one new parser rather than changes throughout the converter, and it is where
//! units are normalised: every distance here is metres, every angle is degrees
//! true, and every position is WGS84.

pub mod from_bgl;
pub mod merge;

use crate::geo::LatLon;

/// Which simulator a package came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SimKind {
    Msfs2020,
    Msfs2024,
    Fsx,
    #[default]
    Unknown,
}

impl SimKind {
    pub fn label(self) -> &'static str {
        match self {
            SimKind::Msfs2020 => "MSFS 2020",
            SimKind::Msfs2024 => "MSFS 2024",
            SimKind::Fsx => "FSX/P3D",
            SimKind::Unknown => "unknown",
        }
    }
}

/// Ground surface material, normalised across simulators.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Surface {
    #[default]
    Concrete,
    Cement,
    Asphalt,
    Bituminous,
    Tarmac,
    Macadam,
    OilTreated,
    Brick,
    SteelMats,
    Planks,
    Grass,
    Dirt,
    Clay,
    Sand,
    Shale,
    Coral,
    Gravel,
    Snow,
    Ice,
    Water,
    Transparent,
    Unknown,
}

impl Surface {
    pub fn is_water(&self) -> bool {
        matches!(self, Surface::Water)
    }

    pub fn is_hard(&self) -> bool {
        matches!(
            self,
            Surface::Concrete
                | Surface::Cement
                | Surface::Asphalt
                | Surface::Bituminous
                | Surface::Tarmac
                | Surface::Macadam
        )
    }
}

/// Light intensity, as MSFS grades it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LightLevel {
    #[default]
    None,
    Low,
    Medium,
    High,
}

impl LightLevel {
    pub fn is_on(self) -> bool {
        !matches!(self, LightLevel::None)
    }
}

/// Approach lighting system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlsSystem {
    #[default]
    None,
    Odals,
    Malsf,
    Malsr,
    Mals,
    Ssalf,
    Ssalr,
    Ssals,
    Sals,
    Salsf,
    Alsf1,
    Alsf2,
    Rail,
    Calvert,
    Calvert2,
}

/// Visual glideslope indicator type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VasiKind {
    #[default]
    None,
    Vasi,
    Papi2,
    Papi4,
    TriColor,
    PulsatingVasi,
    TVasi,
    Ball,
    Apap,
}

/// Which side of the runway a light unit sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// A visual glideslope indicator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vasi {
    pub kind: VasiKind,
    pub side: Side,
    /// Glide path angle in degrees.
    pub angle: f32,
}

/// Approach lighting at one runway end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ApproachLights {
    pub system: AlsSystem,
    pub reil: bool,
    pub touchdown: bool,
    pub end_lights: bool,
}

/// Which painted markings a runway carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunwayMarkings {
    pub edges: bool,
    pub threshold: bool,
    pub fixed_distance: bool,
    pub touchdown: bool,
    pub dashes: bool,
    pub ident: bool,
    pub precision: bool,
    /// International rather than US marking style.
    pub alternate: bool,
    pub edge_pavement: bool,
}

/// One end of a runway.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunwayEnd {
    /// Full name such as `09L`, `27` or `NE`.
    pub name: String,
    pub closed: bool,
    pub stol: bool,
    /// Displaced threshold, part of the runway length.
    pub displaced_m: f32,
    /// Blast pad, added beyond the runway end.
    pub blast_pad_m: f32,
    pub overrun_m: f32,
    pub vasi: Vec<Vasi>,
    pub approach_lights: Option<ApproachLights>,
    pub takeoff: bool,
    pub landing: bool,
    pub ils_ident: Option<String>,
}

/// A runway, held as a centre point plus heading and length as MSFS stores it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Runway {
    pub centre: LatLon,
    pub elevation_m: f64,
    pub heading_true: f32,
    pub length_m: f32,
    pub width_m: f32,
    pub surface: Surface,
    pub ends: [RunwayEnd; 2],
    pub edge_lights: LightLevel,
    pub centre_lights: LightLevel,
    pub centre_lights_red_end: bool,
    pub markings: RunwayMarkings,
}

/// Helipad marking style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HelipadKind {
    #[default]
    None,
    H,
    Square,
    Circle,
    Medical,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Helipad {
    pub pos: LatLon,
    pub elevation_m: f64,
    pub heading: f32,
    pub length_m: f32,
    pub width_m: f32,
    pub surface: Surface,
    pub kind: HelipadKind,
    pub closed: bool,
    pub transparent: bool,
}

/// Radio service type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComKind {
    Atis,
    Awos,
    Asos,
    Unicom,
    Ctaf,
    Multicom,
    Clearance,
    ClearancePreTaxi,
    RemoteClearance,
    Ground,
    Tower,
    Approach,
    Departure,
    Center,
    Fss,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Com {
    pub kind: ComKind,
    /// Frequency in kilohertz.
    pub freq_khz: u32,
    pub name: String,
}

/// What a taxi network node represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NodeKind {
    #[default]
    Normal,
    HoldShort,
    IlsHoldShort,
    /// Hold short without the painted bar (grass and gravel taxiways).
    HoldShortNoDraw,
    IlsHoldShortNoDraw,
    /// A parking stand, which is also a node in the MSFS index space.
    Parking,
}

impl NodeKind {
    pub fn is_hold_short(self) -> bool {
        matches!(
            self,
            NodeKind::HoldShort | NodeKind::IlsHoldShort | NodeKind::HoldShortNoDraw | NodeKind::IlsHoldShortNoDraw
        )
    }

    pub fn is_ils(self) -> bool {
        matches!(self, NodeKind::IlsHoldShort | NodeKind::IlsHoldShortNoDraw)
    }

    pub fn draws_bar(self) -> bool {
        matches!(self, NodeKind::HoldShort | NodeKind::IlsHoldShort)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TaxiNode {
    /// Index in the airport's own node space, as taxi paths reference it.
    pub index: usize,
    pub pos: LatLon,
    pub kind: NodeKind,
    /// Hold-short bars are directional; true when the orientation is reversed.
    pub reverse: bool,
}

/// What a taxi path segment is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PathKind {
    #[default]
    Unknown,
    Taxi,
    Runway,
    Parking,
    Path,
    Closed,
    Vehicle,
    /// MSFS 2024 service road.
    Road,
    /// MSFS 2024 painted line carried on a path.
    PaintedLine,
}

impl PathKind {
    /// Paths aircraft actually taxi on, which become the ATC network.
    pub fn is_aircraft_route(self) -> bool {
        matches!(
            self,
            PathKind::Taxi | PathKind::Runway | PathKind::Parking | PathKind::Path
        )
    }

    /// Paths ground vehicles use, which become X-Plane service roads.
    pub fn is_ground_vehicle(self) -> bool {
        matches!(self, PathKind::Vehicle | PathKind::Road)
    }
}

/// Edge line style along a taxiway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeLine {
    #[default]
    None,
    Solid,
    Dashed,
    SolidDashed,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TaxiPath {
    pub start: usize,
    pub end: usize,
    pub kind: PathKind,
    /// Taxiway name, or the runway name for runway paths.
    pub name: String,
    pub width_m: f32,
    pub surface: Surface,
    pub draw_surface: bool,
    pub centre_line: bool,
    pub centre_line_lit: bool,
    pub left_edge: EdgeLine,
    pub left_edge_lit: bool,
    pub right_edge: EdgeLine,
    pub right_edge_lit: bool,
}

/// Stand category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParkingKind {
    #[default]
    Unknown,
    RampGa,
    RampGaSmall,
    RampGaMedium,
    RampGaLarge,
    RampGaExtra,
    RampCargo,
    RampMilCargo,
    RampMilCombat,
    GateSmall,
    GateMedium,
    GateHeavy,
    GateExtra,
    DockGa,
    Fuel,
    Vehicle,
}

impl ParkingKind {
    pub fn is_gate(self) -> bool {
        matches!(
            self,
            ParkingKind::GateSmall | ParkingKind::GateMedium | ParkingKind::GateHeavy | ParkingKind::GateExtra
        )
    }

    pub fn is_cargo(self) -> bool {
        matches!(self, ParkingKind::RampCargo | ParkingKind::RampMilCargo)
    }

    pub fn is_military(self) -> bool {
        matches!(self, ParkingKind::RampMilCargo | ParkingKind::RampMilCombat)
    }

    /// Stands that hold an aircraft, as opposed to fuel points and vehicle bays.
    pub fn is_aircraft_stand(self) -> bool {
        !matches!(self, ParkingKind::Fuel | ParkingKind::Vehicle | ParkingKind::Unknown)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parking {
    pub index: usize,
    pub pos: LatLon,
    pub heading: f32,
    pub radius_m: f32,
    pub kind: ParkingKind,
    /// Display name such as `A12` or `Parking 5`.
    pub name: String,
    pub airlines: Vec<String>,
    pub has_jetway: bool,
    /// Where the stand's jetway model is anchored and which way it faces.
    pub jetway_base: Option<(LatLon, f32)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Apron {
    pub surface: Surface,
    pub draw: bool,
    pub vertices: Vec<LatLon>,
}

/// Painted ground marking style, as named in the MSFS scenery editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaintedLineKind {
    #[default]
    Default,
    HoldShortForward,
    HoldShortBackward,
    HoldShortTaxiway,
    IlsHoldShort,
    EdgeDashed,
    EdgeSolid,
    EdgeServiceDashed,
    EdgeServiceSolid,
    ServiceDashed,
    WideYellow,
    WideWhite,
    WideRed,
    SlimRed,
    NonMovement,
    NonMovementBack,
    EnhancedCentre,
    EdgeSolidOrtho,
    Other(u16),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaintedLine {
    pub kind: PaintedLineKind,
    pub vertices: Vec<LatLon>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sign {
    pub pos: LatLon,
    pub heading: f32,
    /// MSFS size 1 (smallest) to 5 (largest).
    pub size: u8,
    /// The raw MSFS label, translated to X-Plane syntax during conversion.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tower {
    pub pos: LatLon,
    pub elevation_m: f64,
    pub has_object: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Windsock {
    pub pos: LatLon,
    pub lit: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Beacon {
    pub pos: LatLon,
}

/// A runway, water or helipad start position.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StartPos {
    pub pos: LatLon,
    pub elevation_m: f64,
    pub heading: f32,
    pub runway: String,
    pub is_helipad: bool,
}

/// Where an airport came from, for reports and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceInfo {
    pub file: String,
    pub package: String,
    pub sim: SimKind,
    pub layout: String,
}

/// A complete airport, independent of which simulator described it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Airport {
    pub icao: String,
    pub name: String,
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub region: Option<String>,
    pub datum: LatLon,
    pub elevation_m: f64,
    /// Magnetic variation, negative east and positive west.
    pub magvar: f32,
    pub closed: bool,
    pub flatten: bool,
    pub tower: Option<Tower>,
    pub runways: Vec<Runway>,
    pub helipads: Vec<Helipad>,
    pub starts: Vec<StartPos>,
    pub coms: Vec<Com>,
    pub taxi_nodes: Vec<TaxiNode>,
    pub taxi_paths: Vec<TaxiPath>,
    pub parkings: Vec<Parking>,
    pub aprons: Vec<Apron>,
    pub apron_edge_lights: Vec<Vec<LatLon>>,
    pub painted_lines: Vec<PaintedLine>,
    pub signs: Vec<Sign>,
    pub windsocks: Vec<Windsock>,
    pub beacons: Vec<Beacon>,
    /// True when the source declared a delete record, meaning it replaces a
    /// stock airport rather than adding to one.
    pub replaces_stock: bool,
    pub source: SourceInfo,
    pub warnings: Vec<String>,
}

impl Airport {
    /// A node by its index in the airport's own index space.
    pub fn node(&self, index: usize) -> Option<&TaxiNode> {
        self.taxi_nodes.iter().find(|n| n.index == index)
    }

    /// True when the airport has nothing worth writing out.
    pub fn is_empty(&self) -> bool {
        self.runways.is_empty() && self.helipads.is_empty() && self.taxi_paths.is_empty() && self.parkings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_classification() {
        assert!(Surface::Asphalt.is_hard());
        assert!(Surface::Concrete.is_hard());
        assert!(!Surface::Grass.is_hard());
        assert!(Surface::Water.is_water());
        assert!(!Surface::Ice.is_water());
    }

    #[test]
    fn node_kind_helpers() {
        assert!(NodeKind::IlsHoldShort.is_hold_short());
        assert!(NodeKind::IlsHoldShort.is_ils());
        assert!(NodeKind::IlsHoldShortNoDraw.is_ils());
        assert!(!NodeKind::IlsHoldShortNoDraw.draws_bar());
        assert!(NodeKind::HoldShort.draws_bar());
        assert!(!NodeKind::Normal.is_hold_short());
    }

    #[test]
    fn path_kind_routing() {
        assert!(PathKind::Taxi.is_aircraft_route());
        assert!(PathKind::Runway.is_aircraft_route());
        assert!(!PathKind::Vehicle.is_aircraft_route());
        assert!(PathKind::Vehicle.is_ground_vehicle());
        assert!(PathKind::Road.is_ground_vehicle());
        assert!(!PathKind::Closed.is_ground_vehicle());
    }

    #[test]
    fn parking_classification() {
        assert!(ParkingKind::GateHeavy.is_gate());
        assert!(!ParkingKind::RampGa.is_gate());
        assert!(ParkingKind::RampMilCargo.is_cargo());
        assert!(ParkingKind::RampMilCargo.is_military());
        assert!(ParkingKind::RampGa.is_aircraft_stand());
        assert!(!ParkingKind::Fuel.is_aircraft_stand());
        assert!(!ParkingKind::Vehicle.is_aircraft_stand());
    }

    #[test]
    fn empty_airport_is_reported_empty() {
        let mut ap = Airport::default();
        assert!(ap.is_empty());
        ap.runways.push(Runway::default());
        assert!(!ap.is_empty());
    }

    #[test]
    fn node_lookup_uses_the_index_field_not_position() {
        let ap = Airport {
            taxi_nodes: vec![
                TaxiNode {
                    index: 7,
                    ..Default::default()
                },
                TaxiNode {
                    index: 3,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(ap.node(3).map(|n| n.index), Some(3));
        assert!(ap.node(0).is_none());
    }
}
