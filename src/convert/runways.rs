//! Runways, water runways, helipads and glideslope indicators.

use crate::geo::{destination, LatLon};
use crate::model::{Airport, Runway, Side, Surface};
use crate::xplane::apt::{self, AptAirport};

use super::tables::{als_code, edge_light_code, marking_code, surface_code, vasi_code};
use super::Report;

/// Both runway end positions, primary first.
///
/// MSFS stores the centre, true heading and length; X-Plane wants the two ends.
/// The primary end is the one the heading points away from.
pub fn runway_end_positions(r: &Runway) -> (LatLon, LatLon) {
    let half = r.length_m as f64 / 2.0;
    let h = r.heading_true as f64;
    (destination(r.centre, h + 180.0, half), destination(r.centre, h, half))
}

/// Where a glideslope unit stands: a set distance past the threshold, off to one
/// side of the runway edge, facing the approach.
fn vasi_position(threshold: LatLon, landing_heading: f64, width_m: f64, side: Side) -> LatLon {
    let along = destination(threshold, landing_heading, 300.0);
    let lateral = width_m / 2.0 + 15.0;
    let bearing = match side {
        Side::Left => landing_heading - 90.0,
        Side::Right => landing_heading + 90.0,
    };
    destination(along, bearing, lateral)
}

pub fn build(ap: &Airport, out: &mut AptAirport, report: &mut Report) {
    for r in &ap.runways {
        if !(r.length_m > 1.0 && r.width_m > 0.0 && r.centre.is_valid()) {
            report.dropped("runways with no length or width", 1);
            continue;
        }
        let (p, s) = runway_end_positions(r);
        let names = [r.ends[0].name.clone(), r.ends[1].name.clone()];

        if r.surface == Surface::Water {
            out.water_runways.push(apt::WaterRunway {
                width_m: r.width_m,
                buoys: true,
                ends: [(names[0].clone(), p.lat, p.lon), (names[1].clone(), s.lat, s.lon)],
            });
            report.converted("water runways", 1);
            continue;
        }

        let marking = marking_code(&r.markings);
        let make_end = |i: usize, pos: LatLon| {
            let e = &r.ends[i];
            let al = e.approach_lights.unwrap_or_default();
            apt::RunwayEnd {
                name: e.name.clone(),
                lat: pos.lat,
                lon: pos.lon,
                displaced_m: e.displaced_m.min(r.length_m * 0.9),
                // X-Plane has one "blast pad" length; MSFS splits it from overrun.
                blast_pad_m: e.blast_pad_m.max(e.overrun_m),
                markings: if e.closed { 0 } else { marking },
                approach_lights: als_code(al.system),
                touchdown_lights: al.touchdown,
                reil: u8::from(al.reil),
            }
        };
        let shoulder = if r.markings.edge_pavement {
            if surface_code(&r.surface) == apt::surface::CONCRETE {
                2
            } else {
                1
            }
        } else {
            0
        };
        out.runways.push(apt::Runway {
            width_m: r.width_m,
            surface: surface_code(&r.surface),
            shoulder,
            smoothness: 0.25,
            centre_lights: r.centre_lights.is_on(),
            edge_lights: edge_light_code(r.edge_lights),
            distance_signs: false,
            ends: [make_end(0, p), make_end(1, s)],
        });
        report.converted("runways", 1);

        // Glideslope indicators, one per unit per end.
        for (i, (end_pos, landing_heading)) in [(p, r.heading_true as f64), (s, r.heading_true as f64 + 180.0)]
            .into_iter()
            .enumerate()
        {
            let e = &r.ends[i];
            let threshold = destination(end_pos, landing_heading, e.displaced_m as f64);
            for v in &e.vasi {
                let Some(kind) = vasi_code(v.kind, v.side) else {
                    continue;
                };
                let at = vasi_position(threshold, landing_heading, r.width_m as f64, v.side);
                out.light_objects.push(apt::LightObject {
                    lat: at.lat,
                    lon: at.lon,
                    kind,
                    heading: (landing_heading % 360.0) as f32,
                    angle: v.angle,
                    runway: e.name.clone(),
                    description: format!("{:?} {}", v.kind, e.name),
                });
                report.converted("glideslope indicators", 1);
            }
        }
    }

    for (i, h) in ap.helipads.iter().enumerate() {
        if !(h.length_m > 0.0 && h.width_m > 0.0 && h.pos.is_valid()) {
            report.dropped("helipads with no size", 1);
            continue;
        }
        out.helipads.push(apt::Helipad {
            name: format!("H{}", i + 1),
            lat: h.pos.lat,
            lon: h.pos.lon,
            heading: h.heading,
            length_m: h.length_m,
            width_m: h.width_m,
            surface: if h.transparent {
                apt::surface::TRANSPARENT
            } else {
                surface_code(&h.surface)
            },
            markings: 0,
            shoulder: 0,
            smoothness: 0.25,
            edge_lights: 0,
        });
        report.converted("helipads", 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::inverse;
    use crate::model::*;

    fn runway() -> Runway {
        Runway {
            centre: LatLon::new(25.2528, 55.3644),
            heading_true: 120.0,
            length_m: 4000.0,
            width_m: 60.0,
            surface: Surface::Asphalt,
            ends: [
                RunwayEnd {
                    name: "12L".into(),
                    displaced_m: 300.0,
                    vasi: vec![Vasi {
                        kind: VasiKind::Papi4,
                        side: Side::Left,
                        angle: 3.0,
                    }],
                    approach_lights: Some(ApproachLights {
                        system: AlsSystem::Alsf2,
                        touchdown: true,
                        reil: false,
                        end_lights: true,
                    }),
                    ..Default::default()
                },
                RunwayEnd {
                    name: "30R".into(),
                    closed: true,
                    ..Default::default()
                },
            ],
            markings: RunwayMarkings {
                precision: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn ends_are_the_runway_length_apart_on_the_heading() {
        let (p, s) = runway_end_positions(&runway());
        let (len, brg) = inverse(p, s);
        assert!((len - 4000.0).abs() < 0.05, "{len}");
        assert!(
            (brg - 120.0).abs() < 0.01,
            "primary end must be behind the heading: {brg}"
        );
    }

    #[test]
    fn builds_the_row_with_per_end_details() {
        let ap = Airport {
            runways: vec![runway()],
            ..Default::default()
        };
        let mut out = AptAirport::default();
        let mut report = Report::default();
        build(&ap, &mut out, &mut report);
        let r = &out.runways[0];
        assert_eq!(r.ends[0].markings, 3);
        assert_eq!(r.ends[1].markings, 0, "a closed end loses its markings");
        assert_eq!(r.ends[0].approach_lights, 2);
        assert!(r.ends[0].touchdown_lights);
        assert_eq!(r.ends[0].displaced_m, 300.0);
        assert_eq!(out.light_objects.len(), 1);
        let papi = &out.light_objects[0];
        assert_eq!(papi.kind, 2);
        assert_eq!(papi.heading, 120.0);
        // Left of the runway, on the approach side of the far end.
        let (p, _) = runway_end_positions(&runway());
        let (d, _) = inverse(p, LatLon::new(papi.lat, papi.lon));
        assert!(d > 550.0 && d < 700.0, "PAPI distance from the end {d}");
    }

    #[test]
    fn water_runways_use_their_own_row() {
        let mut r = runway();
        r.surface = Surface::Water;
        let ap = Airport {
            runways: vec![r],
            ..Default::default()
        };
        let mut out = AptAirport::default();
        build(&ap, &mut out, &mut Report::default());
        assert!(out.runways.is_empty());
        assert_eq!(out.water_runways.len(), 1);
    }

    #[test]
    fn degenerate_runways_are_dropped_and_counted() {
        let mut r = runway();
        r.length_m = 0.0;
        let ap = Airport {
            runways: vec![r],
            ..Default::default()
        };
        let mut out = AptAirport::default();
        let mut report = Report::default();
        build(&ap, &mut out, &mut report);
        assert!(out.runways.is_empty());
        assert_eq!(report.dropped.values().sum::<usize>(), 1);
    }
}
