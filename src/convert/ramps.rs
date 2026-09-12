//! Ramp starts (1300/1301) and X-Plane's native animated jetways (1500).
//!
//! MSFS jetways are 3D models animated by the simulator. X-Plane 12 has its own
//! jetways that dock to the aircraft door, driven by one apt.dat row each. Using
//! those means every stand with a jetway in MSFS gets a working, docking jetway
//! in X-Plane without converting the MSFS animation.

use std::collections::HashMap;

use crate::geo::{inverse, Plane};
use crate::model::{Airport, ParkingKind};
use crate::xplane::apt::{AptAirport, Jetway, RampStart};

use super::tables::width_class_from_span;
use super::Report;

fn location(k: ParkingKind) -> &'static str {
    if k.is_gate() {
        "gate"
    } else if k.is_cargo() || k.is_military() {
        "misc"
    } else {
        "tie_down"
    }
}

fn operations(k: ParkingKind) -> &'static str {
    if k.is_gate() {
        "airline"
    } else if k.is_military() {
        "military"
    } else if k.is_cargo() {
        "cargo"
    } else {
        "general_aviation"
    }
}

/// Which X-Plane traffic may use a stand, from its radius.
fn aircraft(k: ParkingKind, radius_m: f32) -> Vec<String> {
    let list: &[&str] = if k == ParkingKind::RampMilCombat {
        &["fighters"]
    } else if k == ParkingKind::DockGa {
        &["props"]
    } else if radius_m >= 26.0 {
        &["heavy", "jets"]
    } else if radius_m >= 16.0 {
        &["jets", "turboprops"]
    } else if radius_m >= 9.0 {
        &["turboprops", "props"]
    } else {
        &["props", "helos"]
    };
    list.iter().map(|s| s.to_string()).collect()
}

pub fn build(ap: &Airport, _plane: &Plane, out: &mut AptAirport, report: &mut Report) {
    let mut names: HashMap<String, usize> = HashMap::new();
    for p in &ap.parkings {
        if !p.kind.is_aircraft_stand() {
            report.dropped("fuel and vehicle stands", 1);
            continue;
        }
        if !p.pos.is_valid() {
            report.dropped("stands with an invalid position", 1);
            continue;
        }
        // X-Plane and WorldEditor both need stand names to be unique.
        let base = if p.name.is_empty() {
            "Stand".to_string()
        } else {
            p.name.clone()
        };
        let n = names.entry(base.clone()).or_default();
        *n += 1;
        let name = if *n == 1 { base } else { format!("{base}-{n}") };

        out.ramp_starts.push(RampStart {
            lat: p.pos.lat,
            lon: p.pos.lon,
            heading: p.heading,
            location: location(p.kind).into(),
            aircraft: aircraft(p.kind, p.radius_m),
            name,
            width: width_class_from_span(p.radius_m * 2.0),
            operations: operations(p.kind).into(),
            airlines: p.airlines.clone(),
        });
        report.converted("ramp starts", 1);

        if let Some((base, _)) = p.jetway_base {
            let (dist, bearing) = inverse(base, p.pos);
            let tunnel = (dist as f32 - p.radius_m * 0.35).clamp(4.0, 35.0);
            out.jetways.push(Jetway {
                lat: base.lat,
                lon: base.lon,
                install_heading: bearing as f32,
                style: 0,
                size: if p.radius_m >= 30.0 {
                    2
                } else if p.radius_m >= 20.0 {
                    1
                } else {
                    0
                },
                parked_tunnel_heading: bearing as f32,
                parked_tunnel_length: tunnel,
                // The cab faces the aircraft's left side, where the doors are.
                parked_cab_heading: (p.heading + 90.0).rem_euclid(360.0),
            });
            report.converted("jetways", 1);
        } else if p.has_jetway {
            report.dropped("jetways without a recorded position", 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::LatLon;
    use crate::model::*;

    fn stand(name: &str, kind: ParkingKind, radius: f32) -> Parking {
        Parking {
            pos: LatLon::new(25.25, 55.36),
            heading: 180.0,
            radius_m: radius,
            kind,
            name: name.into(),
            airlines: vec!["UAE".into()],
            ..Default::default()
        }
    }

    fn run(parkings: Vec<Parking>) -> (AptAirport, Report) {
        let ap = Airport {
            datum: LatLon::new(25.25, 55.36),
            parkings,
            ..Default::default()
        };
        let mut out = AptAirport::default();
        let mut report = Report::default();
        build(&ap, &Plane::new(ap.datum), &mut out, &mut report);
        (out, report)
    }

    #[test]
    fn heavy_gate_maps_to_an_airline_gate() {
        let (out, _) = run(vec![stand("A12", ParkingKind::GateHeavy, 35.0)]);
        let r = &out.ramp_starts[0];
        assert_eq!(r.location, "gate");
        assert_eq!(r.operations, "airline");
        assert_eq!(r.aircraft, vec!["heavy", "jets"]);
        assert_eq!(r.width, 'F');
    }

    #[test]
    fn fuel_stands_are_dropped_and_names_made_unique() {
        let (out, report) = run(vec![
            stand("P1", ParkingKind::RampGa, 8.0),
            stand("P1", ParkingKind::RampGa, 8.0),
            stand("F", ParkingKind::Fuel, 8.0),
        ]);
        assert_eq!(out.ramp_starts.len(), 2);
        assert_eq!(out.ramp_starts[1].name, "P1-2");
        assert_eq!(out.ramp_starts[0].location, "tie_down");
        assert_eq!(report.dropped.get("fuel and vehicle stands"), Some(&1));
    }

    #[test]
    fn a_placed_jetway_becomes_a_native_jetway_pointing_at_the_stand() {
        let mut s = stand("B18", ParkingKind::GateHeavy, 30.0);
        // Jetway base 40 m east of the stand.
        s.jetway_base = Some((crate::geo::destination(s.pos, 90.0, 40.0), 0.0));
        s.has_jetway = true;
        let (out, _) = run(vec![s]);
        let j = &out.jetways[0];
        assert!((j.install_heading - 270.0).abs() < 0.5, "points back west at the stand");
        assert!(j.parked_tunnel_length > 20.0 && j.parked_tunnel_length < 35.0);
        assert_eq!(j.size, 2);
        assert_eq!(j.parked_cab_heading, 270.0);
    }
}
