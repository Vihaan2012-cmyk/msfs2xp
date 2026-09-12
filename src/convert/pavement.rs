//! Pavement: aprons and drawn taxiway surfaces.
//!
//! Aprons are written one polygon per MSFS apron, in the package's own order.
//! Merging them looked attractive, but MSFS 2024 airports supply thousands of
//! overlapping pieces, and a boolean union of that input produces a few giant
//! polygons (O'Hare: one with 72 000 nodes and 4 200 holes) that X-Plane
//! silently fails to draw, leaving most of the airfield as grass. X-Plane paints
//! overlapping pavement in file order without z-fighting, so the pieces stand
//! as they are, layered the way the package layers them.
//!
//! Drawn taxiway surfaces, which the converter builds itself from clean
//! rectangles and discs, are still unioned into outlines (soft surfaces first,
//! concrete last) and written before the aprons, which paint over them.

use std::collections::{BTreeMap, HashMap};

use crate::geo::poly::{self, Ring, Shape};
use crate::geo::Plane;
use crate::model::{Airport, PathKind};
use crate::xplane::apt::{self, surface, AptAirport, Node};

use super::tables::surface_code;
use super::{Options, Report};

/// Paint order for taxiway surfaces: soft ground first, then asphalt, then
/// concrete on top.
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

fn push_shape(out: &mut AptAirport, plane: &Plane, code: u8, name: String, shape: Shape) {
    let mut rings = vec![ring_to_nodes(plane, &oriented(shape.outer, true))];
    rings.extend(shape.holes.into_iter().map(|h| ring_to_nodes(plane, &oriented(h, false))));
    out.pavements.push(apt::Pavement {
        surface: code,
        smoothness: 0.25,
        heading: 0.0,
        name,
        rings,
    });
}

pub fn build(ap: &Airport, plane: &Plane, opts: &Options, out: &mut AptAirport, report: &mut Report) {
    // Drawn taxiway surfaces: a rectangle per segment plus a disc at each
    // junction so that corners are filled rather than notched.
    let mut taxiways: BTreeMap<u8, Vec<Ring>> = BTreeMap::new();
    let nodes: HashMap<usize, (f64, f64)> = ap.taxi_nodes.iter().map(|n| (n.index, plane.to_xy(n.pos))).collect();
    let mut degree: HashMap<usize, (usize, f32, u8)> = HashMap::new();
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
        taxiways.entry(code).or_default().push(quad);
        for n in [p.start, p.end] {
            let e = degree.entry(n).or_insert((0, 0.0, code));
            e.0 += 1;
            e.1 = e.1.max(p.width_m);
        }
    }
    for (n, (deg, width, code)) in degree {
        if deg >= 2 {
            if let Some(&c) = nodes.get(&n) {
                taxiways.entry(code).or_default().push(poly::disc(c, width as f64 / 2.0, 16));
            }
        }
    }
    let mut codes: Vec<u8> = taxiways.keys().copied().collect();
    codes.sort_by_key(|&c| (draw_rank(c), c));
    for code in codes {
        let rings = taxiways.remove(&code).unwrap_or_default();
        let name = surface_name(code);
        report.converted("taxiway surface pieces", rings.len());
        let shapes: Vec<Shape> = if opts.union {
            match poly::union(rings.clone()) {
                Ok(s) => s,
                Err(e) => {
                    report.warn(format!("{name} taxiway union failed ({e}); writing pieces"));
                    rings.into_iter().map(|outer| Shape { outer, holes: vec![] }).collect()
                }
            }
        } else {
            rings.into_iter().map(|outer| Shape { outer, holes: vec![] }).collect()
        };
        for (i, shape) in shapes.into_iter().enumerate() {
            push_shape(out, plane, code, format!("{name} taxiway {}", i + 1), shape);
            report.converted("taxiway surface polygons", 1);
        }
    }

    // Aprons, one polygon each, in the package's order.
    let mut counts: HashMap<u8, usize> = HashMap::new();
    for a in &ap.aprons {
        let code = surface_code(&a.surface);
        if !a.draw || code == surface::TRANSPARENT || code == surface::WATER {
            report.dropped("decals, invisible and water aprons", 1);
            continue;
        }
        let ring = poly::dedup(&a.vertices.iter().map(|v| plane.to_xy(*v)).collect::<Vec<_>>());
        if ring.len() < 3 || poly::signed_area(&ring).abs() < 0.5 {
            report.dropped("degenerate aprons", 1);
            continue;
        }
        let n = counts.entry(code).or_default();
        *n += 1;
        let name = format!("{} {}", surface_name(code), n);
        push_shape(out, plane, code, name, Shape { outer: ring, holes: vec![] });
        report.converted("apron polygons", 1);
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
            ..Default::default()
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
    fn aprons_are_written_one_for_one_in_package_order() {
        let ap = Airport {
            datum: LatLon::new(25.25, 55.36),
            aprons: vec![
                square(25.260, 55.370, 0.001, Surface::Concrete),
                square(25.250, 55.360, 0.001, Surface::Asphalt),
                square(25.2505, 55.3605, 0.001, Surface::Asphalt),
                square(25.255, 55.365, 0.001, Surface::Grass),
            ],
            ..Default::default()
        };
        let (out, report) = run(&ap, true);
        assert_eq!(out.pavements.len(), 4, "overlapping aprons are never merged");
        let order: Vec<u8> = out.pavements.iter().map(|p| p.surface).collect();
        assert_eq!(order, vec![surface::CONCRETE, surface::ASPHALT, surface::ASPHALT, surface::GRASS]);
        assert!(out.pavements.iter().all(|p| p.rings.len() == 1), "no holes");
        assert_eq!(report.converted.get("apron polygons"), Some(&4));
    }

    #[test]
    fn outer_rings_are_counter_clockwise() {
        let mut clockwise = square(25.250, 55.360, 0.001, Surface::Asphalt);
        clockwise.vertices.reverse();
        let ap = Airport {
            datum: LatLon::new(25.25, 55.36),
            aprons: vec![clockwise],
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
            ..Default::default()
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
    fn drawn_taxiways_are_unioned_and_go_under_the_aprons() {
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
            aprons: vec![square(25.251, 55.3601, 0.0005, Surface::Concrete)],
            ..Default::default()
        };
        let (out, _) = run(&ap, true);
        assert_eq!(out.pavements.len(), 2, "one merged taxiway surface and one apron");
        assert!(out.pavements[0].name.contains("taxiway"), "taxiways are painted first");
        assert_eq!(out.pavements[1].surface, surface::CONCRETE);
        let (raw, _) = run(&ap, false);
        assert_eq!(raw.pavements.len(), 4, "without union: two quads, a disc and the apron");
    }
}
