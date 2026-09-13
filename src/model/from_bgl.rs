//! Turning raw BGL records into the neutral airport model.
//!
//! This is where simulator-specific numbering becomes meaning: surface codes
//! become materials, flag words become booleans, and the packed parking name
//! and number become the string a pilot reads on the stand.

use crate::bgl::codec::runway_name;
use crate::bgl::records::raw::{self, RawAirport, Variant};
use crate::geo::LatLon;

use super::*;

/// Surface codes are shared by runways, taxiways, aprons and helipads.
pub fn surface_from_code(code: u8) -> Surface {
    match code {
        0x00 => Surface::Concrete,
        0x01 => Surface::Grass,
        0x02 => Surface::Water,
        0x03 => Surface::Cement,
        0x04 => Surface::Asphalt,
        0x07 => Surface::Clay,
        0x08 => Surface::Snow,
        0x09 => Surface::Ice,
        0x0C => Surface::Dirt,
        0x0D => Surface::Coral,
        0x0E => Surface::Gravel,
        0x0F => Surface::OilTreated,
        0x10 => Surface::SteelMats,
        0x11 => Surface::Bituminous,
        0x12 => Surface::Brick,
        0x13 => Surface::Macadam,
        0x14 => Surface::Planks,
        0x15 => Surface::Sand,
        0x16 => Surface::Shale,
        0x17 => Surface::Tarmac,
        0x3F => Surface::Transparent,
        _ => Surface::Unknown,
    }
}

fn light_from_code(code: u8) -> LightLevel {
    match code {
        1 => LightLevel::Low,
        2 => LightLevel::Medium,
        3 => LightLevel::High,
        _ => LightLevel::None,
    }
}

fn als_from_code(code: u8) -> AlsSystem {
    match code {
        0x01 => AlsSystem::Odals,
        0x02 => AlsSystem::Malsf,
        0x03 => AlsSystem::Malsr,
        0x04 => AlsSystem::Ssalf,
        0x05 => AlsSystem::Ssalr,
        0x06 => AlsSystem::Alsf1,
        0x07 => AlsSystem::Alsf2,
        0x08 => AlsSystem::Rail,
        0x09 => AlsSystem::Calvert,
        0x0A => AlsSystem::Calvert2,
        0x0B => AlsSystem::Mals,
        0x0C => AlsSystem::Sals,
        0x0D => AlsSystem::Ssals,
        0x0E => AlsSystem::Salsf,
        _ => AlsSystem::None,
    }
}

fn vasi_from_code(code: u16) -> VasiKind {
    match code {
        0x01..=0x06 => VasiKind::Vasi,
        0x07 => VasiKind::Papi2,
        0x08 => VasiKind::Papi4,
        0x09 => VasiKind::TriColor,
        0x0A => VasiKind::PulsatingVasi,
        0x0B => VasiKind::TVasi,
        0x0C => VasiKind::Ball,
        0x0D => VasiKind::Apap,
        _ => VasiKind::None,
    }
}

fn com_from_code(code: u16) -> ComKind {
    // P3D v5 re-encoded the same types with a 0x07 prefix.
    match code & 0x00FF {
        0x01 => ComKind::Atis,
        0x02 => ComKind::Multicom,
        0x03 => ComKind::Unicom,
        0x04 => ComKind::Ctaf,
        0x05 => ComKind::Ground,
        0x06 => ComKind::Tower,
        0x07 => ComKind::Clearance,
        0x08 => ComKind::Approach,
        0x09 => ComKind::Departure,
        0x0A => ComKind::Center,
        0x0B => ComKind::Fss,
        0x0C => ComKind::Awos,
        0x0D => ComKind::Asos,
        0x0E => ComKind::ClearancePreTaxi,
        0x0F => ComKind::RemoteClearance,
        _ => ComKind::Unknown,
    }
}

fn parking_kind_from_code(code: u8) -> ParkingKind {
    match code {
        0x01 => ParkingKind::RampGa,
        0x02 => ParkingKind::RampGaSmall,
        0x03 => ParkingKind::RampGaMedium,
        0x04 => ParkingKind::RampGaLarge,
        0x05 => ParkingKind::RampCargo,
        0x06 => ParkingKind::RampMilCargo,
        0x07 => ParkingKind::RampMilCombat,
        0x08 => ParkingKind::GateSmall,
        0x09 => ParkingKind::GateMedium,
        0x0A => ParkingKind::GateHeavy,
        0x0B => ParkingKind::DockGa,
        0x0C => ParkingKind::Fuel,
        0x0D => ParkingKind::Vehicle,
        0x0E => ParkingKind::RampGaExtra,
        0x0F => ParkingKind::GateExtra,
        // MSFS 2024 added a code with no published meaning; it behaves as a gate.
        0x10 => ParkingKind::GateMedium,
        _ => ParkingKind::Unknown,
    }
}

/// The word MSFS prints before the stand number.
fn parking_name_prefix(code: u8) -> &'static str {
    match code {
        0x01 => "Parking",
        0x02 => "N Parking",
        0x03 => "NE Parking",
        0x04 => "E Parking",
        0x05 => "SE Parking",
        0x06 => "S Parking",
        0x07 => "SW Parking",
        0x08 => "W Parking",
        0x09 => "NW Parking",
        0x0A => "Gate",
        0x0B => "Dock",
        0x0C..=0x25 => "", // Gate A..Z: the letter alone is the name
        _ => "",
    }
}

/// The letter for the `GATE_A`..`GATE_Z` name and suffix codes.
fn gate_letter(code: u8) -> Option<char> {
    if (0x0C..=0x25).contains(&code) {
        Some((b'A' + (code - 0x0C)) as char)
    } else {
        None
    }
}

/// Build the stand name MSFS would display, e.g. `A12`, `Gate 4`, `Parking 7B`.
pub fn parking_display_name(name_code: u8, number: u16, suffix_code: u8) -> String {
    let suffix = gate_letter(suffix_code).map(|c| c.to_string()).unwrap_or_default();
    match gate_letter(name_code) {
        Some(letter) => format!("{letter}{number}{suffix}"),
        None => {
            let prefix = parking_name_prefix(name_code);
            if prefix.is_empty() {
                format!("{number}{suffix}")
            } else {
                format!("{prefix} {number}{suffix}")
            }
        }
    }
}

fn node_kind_from_code(code: u8) -> NodeKind {
    match code {
        2 => NodeKind::HoldShort,
        4 => NodeKind::IlsHoldShort,
        5 => NodeKind::HoldShortNoDraw,
        6 => NodeKind::IlsHoldShortNoDraw,
        _ => NodeKind::Normal,
    }
}

fn path_kind_from_code(code: u8) -> PathKind {
    match code {
        1 => PathKind::Taxi,
        2 => PathKind::Runway,
        3 => PathKind::Parking,
        4 => PathKind::Path,
        5 => PathKind::Closed,
        6 => PathKind::Vehicle,
        7 => PathKind::Road,
        8 => PathKind::PaintedLine,
        _ => PathKind::Unknown,
    }
}

fn edge_from_code(code: u8) -> EdgeLine {
    match code {
        1 => EdgeLine::Solid,
        2 => EdgeLine::Dashed,
        3 => EdgeLine::SolidDashed,
        _ => EdgeLine::None,
    }
}

fn helipad_kind_from_code(code: u8) -> HelipadKind {
    match code {
        1 => HelipadKind::H,
        2 => HelipadKind::Square,
        3 => HelipadKind::Circle,
        4 => HelipadKind::Medical,
        _ => HelipadKind::None,
    }
}

/// Painted line styles, indexed in the order the MSFS scenery editor lists them.
///
/// The record stores `(index << 1) | lighted`: odd values only ever occur for
/// the first five styles, which are exactly the markings that carry lights in
/// reality (centrelines, hold-short bars and taxiway edges). Read plainly, the
/// byte would leave O'Hare with almost no runway hold-short lines.
fn painted_line_kind_from_code(code: u16) -> PaintedLineKind {
    match code {
        0 => PaintedLineKind::Default,
        1 => PaintedLineKind::HoldShortForward,
        2 => PaintedLineKind::HoldShortBackward,
        3 => PaintedLineKind::EdgeDashed,
        4 => PaintedLineKind::EdgeSolid,
        5 => PaintedLineKind::EdgeServiceDashed,
        6 => PaintedLineKind::EdgeServiceSolid,
        7 => PaintedLineKind::HoldShortTaxiway,
        8 => PaintedLineKind::IlsHoldShort,
        9 => PaintedLineKind::ServiceDashed,
        10 => PaintedLineKind::WideYellow,
        11 => PaintedLineKind::NonMovement,
        12 => PaintedLineKind::EnhancedCentre,
        13 => PaintedLineKind::WideWhite,
        14 => PaintedLineKind::NonMovementBack,
        15 => PaintedLineKind::EdgeSolidOrtho,
        16 => PaintedLineKind::EdgeSolidOrtho,
        17 => PaintedLineKind::WideRed,
        18 => PaintedLineKind::SlimRed,
        19 => PaintedLineKind::HoldShortForward,
        20 => PaintedLineKind::HoldShortBackward,
        other => PaintedLineKind::Other(other),
    }
}

fn markings_from_flags(flags: u32) -> RunwayMarkings {
    RunwayMarkings {
        edges: flags & (1 << 0) != 0,
        threshold: flags & (1 << 1) != 0,
        fixed_distance: flags & (1 << 2) != 0,
        touchdown: flags & (1 << 3) != 0,
        dashes: flags & (1 << 4) != 0,
        ident: flags & (1 << 5) != 0,
        precision: flags & (1 << 6) != 0,
        edge_pavement: flags & (1 << 7) != 0,
        // Any of the four "alternate" bits means international style markings.
        alternate: flags & ((1 << 13) | (1 << 14) | (1 << 15) | (1 << 21)) != 0,
    }
}

fn runway_end(raw_end: &raw::RawRunwayEnd, marking_flags: u32, primary: bool, pattern_flags: u8) -> RunwayEnd {
    let (closed_bit, stol_bit, takeoff_bit, landing_bit) = if primary {
        (1u32 << 9, 1u32 << 11, 1u8 << 0, 1u8 << 1)
    } else {
        (1u32 << 10, 1u32 << 12, 1u8 << 3, 1u8 << 4)
    };
    let mut vasi = Vec::new();
    for (raw_vasi, side) in [(&raw_end.vasi_left, Side::Left), (&raw_end.vasi_right, Side::Right)] {
        if let Some(v) = raw_vasi {
            let kind = vasi_from_code(v.kind);
            if kind != VasiKind::None {
                vasi.push(Vasi {
                    kind,
                    side,
                    // A zero pitch means the source left it unset; 3 degrees is
                    // the near-universal default and beats a flat glide path.
                    angle: if v.pitch > 0.1 { v.pitch } else { 3.0 },
                });
            }
        }
    }
    RunwayEnd {
        name: runway_name(raw_end.number, raw_end.designator),
        closed: marking_flags & closed_bit != 0,
        stol: marking_flags & stol_bit != 0,
        displaced_m: raw_end.offset_threshold_m.max(0.0),
        blast_pad_m: raw_end.blast_pad_m.max(0.0),
        overrun_m: raw_end.overrun_m.max(0.0),
        vasi,
        approach_lights: raw_end.approach_lights.map(|a| ApproachLights {
            system: als_from_code(a.system),
            reil: a.reil,
            touchdown: a.touchdown,
            end_lights: a.end_lights,
        }),
        // The pattern bits are inverted: a set bit disables the operation.
        takeoff: pattern_flags & takeoff_bit == 0,
        landing: pattern_flags & landing_bit == 0,
        ils_ident: Some(raw_end.ils_ident.clone()).filter(|s| !s.is_empty()),
    }
}

/// Convert one parsed BGL airport record into the neutral model.
pub fn airport_from_raw(raw: RawAirport, file: &str, package: &str) -> Airport {
    let sim = match raw.variant {
        Variant::Msfs2024 => SimKind::Msfs2024,
        Variant::Msfs2020 => SimKind::Msfs2020,
        Variant::Fsx | Variant::P3dV4 | Variant::P3dV5 | Variant::Fs9 => SimKind::Fsx,
        Variant::Unknown => SimKind::Unknown,
    };

    let runways = raw
        .runways
        .iter()
        .map(|r| Runway {
            centre: LatLon::new(r.lat, r.lon),
            elevation_m: r.alt_m,
            heading_true: r.heading_true,
            length_m: r.length_m,
            width_m: r.width_m,
            surface: surface_from_code(r.surface),
            ends: [
                runway_end(&r.primary, r.marking_flags, true, r.pattern_flags),
                runway_end(&r.secondary, r.marking_flags, false, r.pattern_flags),
            ],
            edge_lights: light_from_code(r.light_flags & 0x3),
            centre_lights: light_from_code((r.light_flags >> 2) & 0x3),
            centre_lights_red_end: r.light_flags & 0x20 != 0,
            markings: markings_from_flags(r.marking_flags),
        })
        .collect();

    let mut helipads: Vec<Helipad> = raw
        .helipads
        .iter()
        .map(|h| Helipad {
            pos: LatLon::new(h.lat, h.lon),
            elevation_m: h.alt_m,
            heading: h.heading,
            length_m: h.length_m,
            width_m: h.width_m,
            surface: surface_from_code(h.surface),
            kind: helipad_kind_from_code(h.kind),
            closed: h.closed,
            transparent: h.transparent,
        })
        .collect();

    let starts = raw
        .starts
        .iter()
        .map(|s| StartPos {
            pos: LatLon::new(s.lat, s.lon),
            elevation_m: s.alt_m,
            heading: s.heading,
            runway: runway_name(s.number, s.designator),
            is_helipad: s.kind == 3,
        })
        .collect();

    // Some heliports define a helipad start position but no pad. X-Plane needs
    // a pad to put a helicopter there, so a standard 20 m pad is synthesised.
    if helipads.is_empty() {
        for st in raw.starts.iter().filter(|st| st.kind == 3) {
            helipads.push(Helipad {
                pos: LatLon::new(st.lat, st.lon),
                elevation_m: st.alt_m,
                heading: st.heading,
                length_m: 20.0,
                width_m: 20.0,
                surface: Surface::Concrete,
                kind: HelipadKind::H,
                closed: false,
                transparent: false,
            });
        }
    }

    let coms = raw
        .coms
        .iter()
        .map(|c| Com {
            kind: com_from_code(c.kind),
            freq_khz: c.freq_khz,
            name: c.name.clone(),
        })
        .collect();

    // MSFS numbers taxi points and parking stands in one shared index space:
    // points first in file order, then parkings. Taxi paths reference that
    // space, so the two lists must be concatenated in exactly this order or
    // every path connects to the wrong place.
    let mut taxi_nodes: Vec<TaxiNode> = raw
        .taxi_points
        .iter()
        .enumerate()
        .map(|(i, p)| TaxiNode {
            index: i,
            pos: LatLon::new(p.lat, p.lon),
            kind: node_kind_from_code(p.kind),
            reverse: p.orientation == 2,
        })
        .collect();

    // Jetways identify their stand by number, name code and suffix together:
    // numbers repeat across piers (A12 and B12), and lettered stands share a
    // number (C10, C10A and C10W each have their own jetway).
    let jetway_of = |j: &crate::bgl::records::raw::RawJetway| super::StandJetway {
        base: j
            .placement
            .as_ref()
            .map(|pl| (LatLon::new(pl.lat, pl.lon), pl.heading)),
        model: match (&j.sim_object_title, &j.placement) {
            (Some(title), _) => super::JetwayModel::SimObject(title.clone()),
            (None, Some(pl)) if !pl.guid.is_nil() => super::JetwayModel::Library(pl.guid),
            _ => super::JetwayModel::Unknown,
        },
        spec: None,
    };
    let mut stand_jetways: Vec<Vec<super::StandJetway>> = vec![Vec::new(); raw.parkings.len()];
    let mut unmatched = Vec::new();
    for j in &raw.jetways {
        let key = (j.parking_number, j.parking_name, j.parking_suffix);
        match raw
            .parkings
            .iter()
            .position(|p| (p.number, p.name as u16, p.suffix as u16) == key)
        {
            Some(i) => stand_jetways[i].push(jetway_of(j)),
            None => unmatched.push(j),
        }
    }
    // A jetway whose suffix matches no stand (older files record none) goes
    // to a same-numbered stand, one without a jetway if there is one.
    for j in unmatched {
        let same: Vec<usize> = (0..raw.parkings.len())
            .filter(|&i| (raw.parkings[i].number, raw.parkings[i].name as u16) == (j.parking_number, j.parking_name))
            .collect();
        if let Some(&i) = same.iter().find(|&&i| stand_jetways[i].is_empty()).or(same.first()) {
            stand_jetways[i].push(jetway_of(j));
        }
    }
    let mut parkings = Vec::with_capacity(raw.parkings.len());
    for (i, p) in raw.parkings.iter().enumerate() {
        let index = raw.taxi_points.len() + i;
        taxi_nodes.push(TaxiNode {
            index,
            pos: LatLon::new(p.lat, p.lon),
            kind: NodeKind::Parking,
            reverse: false,
        });
        parkings.push(Parking {
            index,
            pos: LatLon::new(p.lat, p.lon),
            heading: p.heading,
            radius_m: p.radius_m,
            kind: parking_kind_from_code(p.kind),
            name: parking_display_name(p.name, p.number, p.suffix),
            airlines: p.airlines.clone(),
            jetways: std::mem::take(&mut stand_jetways[i]),
        });
    }

    let taxi_paths = raw
        .taxi_paths
        .iter()
        .map(|p| {
            let kind = path_kind_from_code(p.kind);
            let name = if kind == PathKind::Runway {
                runway_name(p.runway_number, p.runway_designator)
            } else {
                raw.taxi_names.get(p.name_index as usize).cloned().unwrap_or_default()
            };
            TaxiPath {
                start: p.start as usize,
                end: p.end as usize,
                kind,
                name,
                width_m: p.width_m,
                surface: surface_from_code(p.surface),
                draw_surface: p.draw_surface,
                centre_line: p.centerline,
                centre_line_lit: p.centerline_lit,
                left_edge: edge_from_code(p.left_edge),
                left_edge_lit: p.left_edge_lit,
                right_edge: edge_from_code(p.right_edge),
                right_edge_lit: p.right_edge_lit,
            }
        })
        .collect();

    let aprons = raw
        .aprons
        .iter()
        .map(|a| Apron {
            // With a material GUID the byte at offset 6 is flags, not a surface;
            // the surface then comes from the material library.
            surface: if a.material.is_some() {
                Surface::Unknown
            } else {
                surface_from_code(a.surface)
            },
            // The MSFS flag byte does not reliably mark decals (O'Hare's base
            // asphalt and Dubai's use different bits), so overlays are found by
            // material name when the materials are resolved.
            draw: a.draw_surface,
            vertices: a.vertices.iter().map(|&(lat, lon)| LatLon::new(lat, lon)).collect(),
            material_guid: a.material,
            material_name: None,
            tint: a.tint,
            flags: a.flags,
            uv_scale: a.uv_scale,
            uv_rotation: a.uv_rotation,
            priority: a.priority,
            decal_texture: None,
        })
        .collect();

    let painted_lines = raw
        .painted_lines
        .iter()
        .map(|l| PaintedLine {
            kind: painted_line_kind_from_code(l.kind >> 1),
            lit: l.kind & 1 == 1,
            vertices: l.vertices.iter().map(|&(lat, lon)| LatLon::new(lat, lon)).collect(),
            material_guid: l.material,
            material_name: None,
        })
        .collect();

    let signs = raw
        .signs
        .iter()
        .map(|s| Sign {
            pos: LatLon::new(s.lat, s.lon),
            heading: s.heading,
            size: s.size.clamp(1, 5),
            label: s.label.clone(),
        })
        .collect();

    // A tower position equal to the airport reference means it was never set.
    // MSFS 2024 leaves the field zeroed, which decodes to 90N 180W, so the
    // tower must also be near the airport.
    let tower_pos = LatLon::new(raw.tower_lat, raw.tower_lon);
    let datum = LatLon::new(raw.lat, raw.lon);
    let near = tower_pos.is_valid() && datum.is_valid() && crate::geo::inverse(datum, tower_pos).0 < 10_000.0;
    let tower = if near && (raw.tower_alt_m - raw.alt_m).abs() > 0.5 {
        Some(Tower {
            pos: tower_pos,
            elevation_m: raw.tower_alt_m,
            has_object: raw.has_tower_obj,
        })
    } else {
        None
    };

    Airport {
        icao: raw.ident.clone(),
        name: raw.name.clone(),
        city: None,
        state: None,
        country: None,
        region: Some(raw.region.clone()).filter(|s| !s.is_empty()),
        datum: LatLon::new(raw.lat, raw.lon),
        elevation_m: raw.alt_m,
        magvar: raw.magvar,
        closed: raw.closed,
        flatten: false,
        tower,
        runways,
        helipads,
        starts,
        coms,
        taxi_nodes,
        taxi_paths,
        parkings,
        aprons,
        apron_edge_lights: raw
            .apron_edge_lights
            .iter()
            .map(|s| LightString {
                name: s.name.clone(),
                points: s.vertices.iter().map(|&(lat, lon)| LatLon::new(lat, lon)).collect(),
            })
            .collect(),
        painted_lines,
        signs,
        windsocks: Vec::new(),
        beacons: Vec::new(),
        replaces_stock: raw.delete_airport,
        source: SourceInfo {
            file: file.to_string(),
            package: package.to_string(),
            sim,
            layout: raw.variant.label().to_string(),
        },
        warnings: raw.warnings.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgl::records::raw::{RawJetway, RawParking, RawTaxiPath, RawTaxiPoint};

    #[test]
    fn surface_codes_map_to_materials() {
        assert_eq!(surface_from_code(0x04), Surface::Asphalt);
        assert_eq!(surface_from_code(0x00), Surface::Concrete);
        assert_eq!(surface_from_code(0x01), Surface::Grass);
        assert_eq!(surface_from_code(0x02), Surface::Water);
        assert_eq!(surface_from_code(0x7E), Surface::Unknown);
    }

    #[test]
    fn parking_names_match_the_simulator() {
        // GATE_A (0x0C) number 12 with no suffix.
        assert_eq!(parking_display_name(0x0C, 12, 0x00), "A12");
        // GATE_A number 12 with suffix GATE_C (0x0E).
        assert_eq!(parking_display_name(0x0C, 12, 0x0E), "A12C");
        assert_eq!(parking_display_name(0x01, 5, 0x00), "Parking 5");
        assert_eq!(parking_display_name(0x0A, 4, 0x00), "Gate 4");
        assert_eq!(parking_display_name(0x03, 2, 0x00), "NE Parking 2");
        assert_eq!(parking_display_name(0x25, 1, 0x00), "Z1");
    }

    #[test]
    fn pattern_flags_are_inverted() {
        // A clear bit means the operation is allowed.
        let end = runway_end(&raw::RawRunwayEnd::default(), 0, true, 0b0000_0000);
        assert!(end.takeoff && end.landing);
        let end = runway_end(&raw::RawRunwayEnd::default(), 0, true, 0b0000_0011);
        assert!(!end.takeoff && !end.landing);
        // The secondary end uses different bits.
        let end = runway_end(&raw::RawRunwayEnd::default(), 0, false, 0b0001_1000);
        assert!(!end.takeoff && !end.landing);
    }

    #[test]
    fn closed_and_stol_flags_pick_the_right_end() {
        let primary = runway_end(&raw::RawRunwayEnd::default(), 1 << 9, true, 0);
        assert!(primary.closed);
        let secondary = runway_end(&raw::RawRunwayEnd::default(), 1 << 9, false, 0);
        assert!(!secondary.closed);
        let secondary = runway_end(&raw::RawRunwayEnd::default(), 1 << 10, false, 0);
        assert!(secondary.closed);
    }

    #[test]
    fn vasi_without_a_pitch_gets_a_three_degree_default() {
        let end_raw = raw::RawRunwayEnd {
            vasi_right: Some(raw::RawVasi {
                kind: 0x08,
                pitch: 0.0,
                ..Default::default()
            }),
            ..Default::default()
        };
        let end = runway_end(&end_raw, 0, true, 0);
        assert_eq!(end.vasi.len(), 1);
        assert_eq!(end.vasi[0].kind, VasiKind::Papi4);
        assert_eq!(end.vasi[0].side, Side::Right);
        assert_eq!(end.vasi[0].angle, 3.0);
    }

    #[test]
    fn parkings_continue_the_taxi_point_index_space() {
        let raw = RawAirport {
            ident: "OMDB".into(),
            variant: Variant::Msfs2020,
            taxi_points: vec![RawTaxiPoint::default(); 3],
            parkings: vec![
                RawParking {
                    number: 1,
                    name: 0x0C,
                    kind: 0x0A,
                    ..Default::default()
                },
                RawParking {
                    number: 2,
                    name: 0x0C,
                    kind: 0x0A,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let ap = airport_from_raw(raw, "f.bgl", "pkg");
        assert_eq!(ap.taxi_nodes.len(), 5);
        assert_eq!(ap.parkings[0].index, 3, "first parking follows the last taxi point");
        assert_eq!(ap.parkings[1].index, 4);
        assert_eq!(ap.taxi_nodes[3].kind, NodeKind::Parking);
        assert_eq!(ap.parkings[0].name, "A1");
    }

    #[test]
    fn jetways_attach_by_stand_number_and_name() {
        let raw = RawAirport {
            variant: Variant::Msfs2020,
            parkings: vec![
                RawParking {
                    number: 9,
                    name: 0x0C, // A9
                    ..Default::default()
                },
                RawParking {
                    number: 9,
                    name: 0x0D, // B9
                    ..Default::default()
                },
            ],
            jetways: vec![RawJetway {
                parking_number: 9,
                parking_name: 0x0D,
                placement: None,
                ..Default::default()
            }],
            ..Default::default()
        };
        let ap = airport_from_raw(raw, "f.bgl", "pkg");
        assert!(ap.parkings[0].jetways.is_empty(), "A9 has no jetway");
        assert_eq!(ap.parkings[1].jetways.len(), 1, "B9 has the jetway");
        assert!(ap.parkings[1].jetways[0].base.is_none());
    }

    #[test]
    fn jetways_match_the_stand_suffix() {
        let stand = |suffix| RawParking {
            number: 10,
            name: 0x0E,
            suffix,
            ..Default::default()
        };
        let jetway = |suffix| RawJetway {
            parking_number: 10,
            parking_name: 0x0E,
            parking_suffix: suffix,
            ..Default::default()
        };
        let raw = RawAirport {
            variant: Variant::Msfs2020,
            // C10, C10A and C10W.
            parkings: vec![stand(0), stand(12), stand(34)],
            jetways: vec![jetway(34), jetway(0), jetway(12)],
            ..Default::default()
        };
        let ap = airport_from_raw(raw, "f.bgl", "pkg");
        let counts: Vec<usize> = ap.parkings.iter().map(|p| p.jetways.len()).collect();
        assert_eq!(counts, vec![1, 1, 1], "each stand keeps its own jetway");
    }

    #[test]
    fn runway_paths_take_their_name_from_the_runway_number() {
        let raw = RawAirport {
            variant: Variant::Msfs2020,
            taxi_names: vec!["".into(), "A".into()],
            taxi_paths: vec![
                RawTaxiPath {
                    kind: 2, // runway
                    name_index: 12,
                    runway_number: 12,
                    runway_designator: 1,
                    ..Default::default()
                },
                RawTaxiPath {
                    kind: 1, // taxi
                    name_index: 1,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let ap = airport_from_raw(raw, "f.bgl", "pkg");
        assert_eq!(ap.taxi_paths[0].name, "12L");
        assert_eq!(ap.taxi_paths[1].name, "A");
    }

    #[test]
    fn tower_is_dropped_when_it_sits_at_the_airport_datum() {
        let raw = RawAirport {
            variant: Variant::Msfs2020,
            lat: 25.25,
            lon: 55.36,
            alt_m: 19.0,
            tower_lat: 25.25,
            tower_lon: 55.36,
            tower_alt_m: 19.0,
            ..Default::default()
        };
        assert!(airport_from_raw(raw, "f.bgl", "pkg").tower.is_none());
    }

    #[test]
    fn tower_is_dropped_when_its_position_is_blank() {
        let raw = RawAirport {
            variant: Variant::Msfs2020,
            lat: 25.25,
            lon: 55.36,
            alt_m: 19.0,
            tower_lat: 90.0,
            tower_lon: -180.0,
            tower_alt_m: 19.0,
            ..Default::default()
        };
        assert!(airport_from_raw(raw, "f.bgl", "pkg").tower.is_none());
    }

    #[test]
    fn a_helipad_start_without_a_pad_gets_one() {
        let raw = RawAirport {
            variant: Variant::Msfs2024,
            starts: vec![crate::bgl::records::raw::RawStart {
                kind: 3,
                lat: 25.1413,
                lon: 55.1852,
                heading: 45.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        let ap = airport_from_raw(raw, "f.bgl", "pkg");
        assert_eq!(ap.helipads.len(), 1);
        assert_eq!(ap.helipads[0].heading, 45.0);
    }

    #[test]
    fn painted_line_type_byte_carries_a_lighted_bit() {
        let raw = RawAirport {
            variant: Variant::Msfs2024,
            painted_lines: vec![
                crate::bgl::records::raw::RawPaintedLine {
                    kind: 5,
                    vertices: vec![(25.0, 55.0), (25.001, 55.0)],
                    ..Default::default()
                },
                crate::bgl::records::raw::RawPaintedLine {
                    kind: 36,
                    vertices: vec![(25.0, 55.0), (25.001, 55.0)],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let ap = airport_from_raw(raw, "f.bgl", "pkg");
        assert_eq!(ap.painted_lines[0].kind, PaintedLineKind::HoldShortBackward);
        assert!(ap.painted_lines[0].lit);
        assert_eq!(ap.painted_lines[1].kind, PaintedLineKind::SlimRed);
        assert!(!ap.painted_lines[1].lit);
    }

    #[test]
    fn com_types_survive_the_p3d_prefix() {
        assert_eq!(com_from_code(0x0006), ComKind::Tower);
        assert_eq!(com_from_code(0x0706), ComKind::Tower);
        assert_eq!(com_from_code(0x000C), ComKind::Awos);
        assert_eq!(com_from_code(0x00FF), ComKind::Unknown);
    }

    #[test]
    fn marking_flags_decode() {
        let m = markings_from_flags(0b0100_0011);
        assert!(m.edges && m.threshold && m.precision);
        assert!(!m.touchdown && !m.dashes);
        assert!(!m.alternate);
        assert!(markings_from_flags(1 << 13).alternate);
        assert!(markings_from_flags(1 << 21).alternate);
    }
}
