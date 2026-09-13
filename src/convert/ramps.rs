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

/// Reach of X-Plane's jetway tunnels by size code, shortest and longest, in
/// metres (the ranges Laminar's jetway facades assign to each tunnel object).
const TUNNEL_REACH: [(f32, f32); 4] = [(11.0, 23.0), (14.0, 29.0), (17.0, 38.0), (20.0, 47.0)];

/// The X-Plane tunnel whose reach best covers the MSFS jetway's, from parked
/// to fully extended; ties go to the shorter tunnel.
fn tunnel_size(parked: f32, longest: f32) -> u8 {
    let (a, b) = (parked.min(longest), longest.max(parked));
    let mut best = (0u8, -1.0f32);
    for (i, &(lo, hi)) in TUNNEL_REACH.iter().enumerate() {
        let overlap = (b.min(hi) - a.max(lo)).max(0.0);
        if overlap > best.1 {
            best = (i as u8, overlap);
        }
    }
    best.0
}

/// X-Plane jetway style: 0 and 1 are the first cab design, solid and glass;
/// 2 and 3 the second (the `#cabin` codes of Laminar's jetway facades).
fn style_code(glass: bool, second_design: bool) -> u8 {
    u8::from(second_design) * 2 + u8::from(glass)
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

        for j in &p.jetways {
            let Some((base, placed_heading)) = j.base else {
                report.dropped("jetways without a recorded position", 1);
                continue;
            };
            let (dist, bearing) = inverse(base, p.pos);
            let (heading, parked, longest, style) = match &j.spec {
                // The MSFS model's parked pose: its tunnel points `angle_deg`
                // from the model's forward axis towards its left, and the
                // placement heading turns forward to that bearing.
                Some(s) => (
                    placed_heading - s.angle_deg,
                    s.rest_m,
                    s.reach_m.1,
                    style_code(s.glass, s.second_design),
                ),
                // A model that could not be measured: Asobo's template parks
                // the tunnel along the model's left (+X), which fits 182 of
                // O'Hare's 190 placements; if that points away from the stand,
                // point at the stand. Parked at a typical length, able to
                // reach the stand.
                None => {
                    let template = (placed_heading - 90.0).rem_euclid(360.0);
                    let off = ((template - bearing as f32).rem_euclid(360.0) - 180.0).abs();
                    let heading = if off >= 90.0 { template } else { bearing as f32 };
                    (heading, 16.0, (dist as f32).max(26.0), 0)
                }
            };
            let heading = heading.rem_euclid(360.0);
            let size = tunnel_size(parked, longest);
            let (shortest, longest) = TUNNEL_REACH[size as usize];
            out.jetways.push(Jetway {
                lat: base.lat,
                lon: base.lon,
                install_heading: heading,
                style,
                size,
                parked_tunnel_heading: heading,
                parked_tunnel_length: parked.clamp(shortest, longest),
                // MSFS parks the cab in line with the tunnel.
                parked_cab_heading: heading,
            });
            report.converted("jetways", 1);
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
    fn a_measured_jetway_keeps_its_parked_pose() {
        let mut s = stand("B18", ParkingKind::GateHeavy, 30.0);
        s.jetways.push(StandJetway {
            base: Some((crate::geo::destination(s.pos, 90.0, 40.0), 180.0)),
            spec: Some(JetwaySpec {
                rest_m: 16.5,
                reach_m: (14.6, 32.5),
                angle_deg: 90.0,
                glass: true,
                second_design: false,
            }),
            ..Default::default()
        });
        let (out, _) = run(vec![s]);
        let j = &out.jetways[0];
        assert_eq!(j.install_heading, 90.0, "heading 180 turns the model's +X to bearing 90");
        assert_eq!(j.parked_tunnel_heading, 90.0);
        assert_eq!(j.parked_cab_heading, 90.0, "cab in line with the tunnel");
        assert_eq!(j.size, 2, "a 16.5-32.5 m reach fits the 17-38 m tunnel best");
        assert_eq!(j.style, 1, "glass, first cab design");
        assert_eq!(j.parked_tunnel_length, 17.0, "kept inside the tunnel's range");
    }

    #[test]
    fn every_jetway_of_a_stand_is_written() {
        let mut s = stand("M17", ParkingKind::GateHeavy, 35.0);
        // Two unmeasured jetways 30 m east: placed at heading 0 the template's
        // +X points west, at the stand; placed at heading 180 it would point
        // away, so the stand's bearing is used instead.
        for heading in [0.0, 180.0] {
            s.jetways.push(StandJetway {
                base: Some((crate::geo::destination(s.pos, 90.0, 30.0), heading)),
                ..Default::default()
            });
        }
        let (out, report) = run(vec![s]);
        assert_eq!(out.jetways.len(), 2);
        assert_eq!(report.converted.get("jetways"), Some(&2));
        for j in &out.jetways {
            assert!((j.install_heading - 270.0).abs() < 0.5, "points west, at the stand: {}", j.install_heading);
            assert_eq!(j.size, 1);
        }
    }

    #[test]
    fn tunnel_sizes_follow_the_msfs_reach() {
        assert_eq!(tunnel_size(12.3, 29.4), 1, "O'Hare's short jetway");
        assert_eq!(tunnel_size(16.5, 32.5), 2, "O'Hare's standard jetway");
        assert_eq!(tunnel_size(20.1, 38.6), 3, "O'Hare's second design");
        assert_eq!(tunnel_size(8.0, 10.0), 0);
        assert_eq!(style_code(false, false), 0);
        assert_eq!(style_code(true, true), 3);
    }
}
