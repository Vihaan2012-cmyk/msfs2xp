//! Pavement: aprons and drawn taxiway surfaces.
//!
//! Modern MSFS airports describe their ground as tens of thousands of small,
//! overlapping apron polygons (Dubai has about 25 000). Written one-for-one they
//! would z-fight and swamp WorldEditor, so they are unioned per X-Plane surface
//! into a handful of large polygons with holes. Draw order matters in X-Plane
//! (later pavement paints over earlier), so soft surfaces go down first.

use std::collections::BTreeMap;

use crate::geo::poly::{self, Ring, Shape};
use crate::geo::Plane;
use crate::model::{Airport, PathKind};
use crate::xplane::apt::{self, surface, AptAirport, Node};

use super::tables::surface_code;
use super::{Options, Report};

/// Paint order: soft ground first, then asphalt, then concrete on top.
fn draw_rank(code: u8) -> u8 {
    match code {
        surface::GRASS | surface::DIRT | surface::GRAVEL | surface::DRY_LAKEBED => 0,
        surface::SNOW_ICE => 1,
        surface::ASPHALT | 20..=38 => 2,
        surface::CONCRETE | 50..=57 => 3,
        _ => 4,
    }
}

fn surface_name(code: u8) -> &'static str {
    match code {
        surface::ASPHALT => "Asphalt",
        surface::CONCRETE => "Concrete",
        surface::GRASS => "Grass",
        surface::DIRT => "Dirt",
        surface::GRAVEL => "Gravel",
        surface::SNOW_ICE => "Snow",
        _ => "Pavement",
    }
}

fn ring_to_nodes(plane: &Plane, ring: &Ring) -> Vec<Node> {
    ring.iter()
        .map(|&(x, y)| {
            let p = plane.to_latlon(x, y);
            Node::at(p.lat, p.lon)
        })
        .collect()
}

/// X-Plane expects outer boundaries counter-clockwise and holes clockwise.
fn oriented(mut ring: Ring, ccw: bool) -> Ring {
    if (poly::signed_area(&ring) > 0.0) != ccw {
        ring.reverse();
    }
    ring
}

pub fn build(ap: &Airport, plane: &Plane, opts: &Options, out: &mut AptAirport, report: &mut Report) {
    let mut groups: BTreeMap<u8, Vec<Ring>> = BTreeMap::new();

    for a in &ap.aprons {
        let code = surface_code(&a.surface);
        if !a.draw || code == surface::TRANSPARENT || code == surface::WATER {
            report.dropped("invisible or water aprons", 1);
            continue;
        }
        let ring = poly::dedup(&a.vertices.iter().map(|v| plane.to_xy(*v)).collect::<Vec<_>>());
        if ring.len() < 3 || poly::signed_area(&ring).abs() < 0.5 {
            report.dropped("degenerate aprons", 1);
            continue;
        }
        groups.entry(code).or_default().push(ring);
    }

    // Drawn taxiway surfaces: a rectangle per segment plus a disc at each
    // junction so that corners are filled rather than notched.
    let nodes: std::collections::HashMap<usize, (f64, f64)> =
        ap.taxi_nodes.iter().map(|n| (n.index, plane.to_xy(n.pos))).collect();
    let mut degree: std::collections::HashMap<usize, (usize, f32, u8)> = std::collections::HashMap::new();
    for p in &ap.taxi_paths {
        let paints = p.draw_surface && !matches!(p.kind, PathKind::Runway | PathKind::PaintedLine | PathKind::Unknown);
        if !paints || p.width_m <= 0.0 {
            continue;
        }
        let code = surface_code(&p.surface);
        if code == surface::TRANSPARENT {
            continue;
        }
        let (Some(&a), Some(&b)) = (nodes.get(&p.start), nodes.get(&p.end)) else {
            report.dropped("taxiway surfaces with missing nodes", 1);
            continue;
        };
        let quad = poly::segment_quad(a, b, p.width_m as f64);
        if quad.is_empty() {
            continue;
        }
        groups.entry(code).or_default().push(quad);
        for n in [p.start, p.end] {
            let e = degree.entry(n).or_insert((0, 0.0, code));
            e.0 += 1;
            e.1 = e.1.max(p.width_m);
        }
    }
    for (n, (deg, width, code)) in degree {
        if deg >= 2 {
            if let Some(&c) = nodes.get(&n) {
                groups
                    .entry(code)
                    .or_default()
                    .push(poly::disc(c, width as f64 / 2.0, 16));
            }
        }
    }

    let mut codes: Vec<u8> = groups.keys().copied().collect();
    codes.sort_by_key(|&c| (draw_rank(c), c));
    for code in codes {
        let rings = groups.remove(&code).unwrap_or_default();
        let input = rings.len();
        let shapes: Vec<Shape> = if opts.union {
            match poly::union(rings.clone()) {
                Ok(s) => s,
                Err(e) => {
                    report.warn(format!(
                        "{} pavement union failed ({e}); writing pieces",
                        surface_name(code)
                    ));
                    rings.into_iter().map(|outer| Shape { outer, holes: vec![] }).collect()
                }
            }
        } else {
            rings.into_iter().map(|outer| Shape { outer, holes: vec![] }).collect()
        };
        report.converted(
            &format!("{} pavement pieces merged", surface_name(code).to_lowercase()),
            input,
        );
        for (i, shape) in shapes.into_iter().enumerate() {
            let mut out_rings = vec![ring_to_nodes(plane, &oriented(shape.outer, true))];
            out_rings.extend(
                shape
                    .holes
                    .into_iter()
                    .map(|h| ring_to_nodes(plane, &oriented(h, false))),
            );
            out.pavements.push(apt::Pavement {
                surface: code,
                smoothness: 0.25,
                heading: 0.0,
                name: format!("{} {}", surface_name(code), i + 1),
                rings: out_rings,
            });
            report.converted("pavement polygons", 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::LatLon;
    use crate::model::*;

    fn square(lat: f64, lon: f64, d: f64, s: Surface) -> Apron {
        Apron {
            surface: s,
            draw: true,
            vertices: vec![
                LatLon::new(lat, lon),
                LatLon::new(lat + d, lon),
                LatLon::new(lat + d, lon + d),
                LatLon::new(lat, lon + d),
            ],
        }
    }

    fn run(ap: &Airport, union: bool) -> (AptAirport, Report) {
        let plane = Plane::new(ap.datum);
        let mut out = AptAirport::default();
        let mut report = Report::default();
        let opts = Options {
            union,
            ..Default::default()
        };
        build(ap, &plane, &opts, &mut out, &mut report);
        (out, report)
    }

    #[test]
    fn overlapping_aprons_of_one_surface_merge() {
        let ap = Airport {
            datum: LatLon::new(25.25, 55.36),
            aprons: vec![
                square(25.250, 55.360, 0.001, Surface::Asphalt),
                square(25.2505, 55.3605, 0.001, Surface::Asphalt),
                square(25.260, 55.370, 0.001, Surface::Concrete),
            ],
            ..Default::default()
        };
        let (out, _) = run(&ap, true);
        assert_eq!(out.pavements.len(), 2, "one merged asphalt polygon and one concrete");
        assert_eq!(
            out.pavements[0].surface,
            surface::ASPHALT,
            "asphalt is painted before concrete"
        );
        assert_eq!(out.pavements[1].surface, surface::CONCRETE);
        let (raw, _) = run(&ap, false);
        assert_eq!(raw.pavements.len(), 3);
    }

    #[test]
    fn outer_rings_are_counter_clockwise() {
        let ap = Airport {
            datum: LatLon::new(25.25, 55.36),
            aprons: vec![square(25.250, 55.360, 0.001, Surface::Asphalt)],
            ..Default::default()
        };
        let (out, _) = run(&ap, true);
        let plane = Plane::new(ap.datum);
        let ring: Ring = out.pavements[0].rings[0]
            .iter()
            .map(|n| plane.to_xy(LatLon::new(n.lat, n.lon)))
            .collect();
        assert!(poly::signed_area(&ring) > 0.0);
    }

    #[test]
    fn invisible_and_degenerate_aprons_are_dropped() {
        let mut hidden = square(25.25, 55.36, 0.001, Surface::Asphalt);
        hidden.draw = false;
        let sliver = Apron {
            surface: Surface::Asphalt,
            draw: true,
            vertices: vec![LatLon::new(25.25, 55.36), LatLon::new(25.2501, 55.36)],
        };
        let ap = Airport {
            datum: LatLon::new(25.25, 55.36),
            aprons: vec![hidden, sliver, square(25.25, 55.36, 0.001, Surface::Transparent)],
            ..Default::default()
        };
        let (out, report) = run(&ap, true);
        assert!(out.pavements.is_empty());
        assert_eq!(report.dropped.values().sum::<usize>(), 3);
    }

    #[test]
    fn drawn_taxiways_become_one_surface() {
        let n = |index, lat| TaxiNode {
            index,
            pos: LatLon::new(lat, 55.36),
            ..Default::default()
        };
        let path = |start, end| TaxiPath {
            start,
            end,
            kind: PathKind::Taxi,
            width_m: 23.0,
            surface: Surface::Asphalt,
            draw_surface: true,
            ..Default::default()
        };
        let ap = Airport {
            datum: LatLon::new(25.25, 55.36),
            taxi_nodes: vec![n(0, 25.250), n(1, 25.252), n(2, 25.254)],
            taxi_paths: vec![path(0, 1), path(1, 2)],
            ..Default::default()
        };
        let (out, _) = run(&ap, true);
        assert_eq!(
            out.pavements.len(),
            1,
            "two segments and a junction disc merge into one"
        );
    }
}
