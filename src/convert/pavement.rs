//! Pavement: aprons and drawn taxiway surfaces.
//!
//! Aprons: MSFS 2024 airports supply thousands of overlapping pieces (O'Hare:
//! 18 623), each painted over the last. An unbounded union of them produced a
//! few giant polygons (one with 72 000 nodes and 4 200 holes) that X-Plane
//! silently fails to draw, leaving most of the airfield as grass. So pieces are
//! merged only where it is safe and bounded: the same surface at the same MSFS
//! draw priority, and only with pieces whose centre falls in the same 200 m
//! tile. The groups are painted in MSFS priority order (a signed value, so -1
//! goes under 0), groups of equal priority in the package's order.
//!
//! Drawn taxiway surfaces, which the converter builds itself from clean
//! rectangles and discs, are still unioned into outlines (soft surfaces first,
//! concrete last) and written before the aprons, which paint over them.

use std::collections::{BTreeMap, HashMap};

use crate::geo::poly::{self, Ring, Shape};
use crate::geo::Plane;
use crate::model::{Airport, PathKind};
use crate::xplane::apt::{self, surface, AptAirport, Node};

use super::tables::shaded_surface_code;
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
        surface::ASPHALT | 20..=38 => "Asphalt",
        surface::CONCRETE | 50..=57 => "Concrete",
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
        let code = shaded_surface_code(&p.surface, p.brightness);
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

    // Aprons: (priority, surface) groups, each split into tiles by piece centre.
    const TILE: f64 = 200.0;
    struct Group {
        first: usize,
        priority: i32,
        code: u8,
        tiles: BTreeMap<(i64, i64), Vec<Ring>>,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut group_of: HashMap<(i32, u8), usize> = HashMap::new();
    for (i, a) in ap.aprons.iter().enumerate() {
        let code = shaded_surface_code(&a.surface, a.brightness);
        if !a.draw || code == surface::TRANSPARENT || code == surface::WATER {
            report.dropped("decals, invisible and water aprons", 1);
            continue;
        }
        let ring = poly::dedup(&a.vertices.iter().map(|v| plane.to_xy(*v)).collect::<Vec<_>>());
        if ring.len() < 3 || poly::signed_area(&ring).abs() < 0.5 {
            report.dropped("degenerate aprons", 1);
            continue;
        }
        // MSFS stores the priority as a u32 holding a signed value.
        let priority = a.priority as i32;
        let g = *group_of.entry((priority, code)).or_insert_with(|| {
            groups.push(Group {
                first: i,
                priority,
                code,
                tiles: BTreeMap::new(),
            });
            groups.len() - 1
        });
        let n = ring.len() as f64;
        let (cx, cy) = ring.iter().fold((0.0, 0.0), |acc, p| (acc.0 + p.0 / n, acc.1 + p.1 / n));
        let tile = ((cx / TILE).floor() as i64, (cy / TILE).floor() as i64);
        groups[g].tiles.entry(tile).or_default().push(ring);
        report.converted("apron pieces", 1);
    }
    groups.sort_by_key(|g| (g.priority, g.first));
    let mut counts: HashMap<u8, usize> = HashMap::new();
    for g in groups {
        let name = surface_name(g.code);
        for (_, rings) in g.tiles {
            let pieces = |rings: Vec<Ring>| -> Vec<Shape> { rings.into_iter().map(|outer| Shape { outer, holes: vec![] }).collect() };
            let shapes = if opts.union && rings.len() > 1 {
                poly::union(rings.clone()).unwrap_or_else(|e| {
                    report.warn(format!("{name} apron union failed ({e}); writing pieces"));
                    pieces(rings)
                })
            } else {
                pieces(rings)
            };
            for shape in shapes {
                let n = counts.entry(g.code).or_default();
                *n += 1;
                push_shape(out, plane, g.code, format!("{name} {n}"), shape);
                report.converted("apron polygons", 1);
            }
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
    fn same_surface_aprons_merge_in_priority_order() {
        let mut first = square(25.250, 55.360, 0.001, Surface::Asphalt);
        first.priority = 5;
        let mut below = square(25.2505, 55.3605, 0.001, Surface::Concrete);
        below.priority = u32::MAX; // -1: painted under priority 0 and up
        let mut overlap = square(25.2505, 55.3605, 0.001, Surface::Asphalt);
        overlap.priority = 5;
        let ap = Airport {
            datum: LatLon::new(25.25, 55.36),
            aprons: vec![first, below, overlap],
            ..Default::default()
        };
        let (out, report) = run(&ap, true);
        assert_eq!(out.pavements.len(), 2, "the two asphalt pieces merge");
        assert_eq!(out.pavements[0].surface, surface::CONCRETE, "priority -1 goes first");
        assert_eq!(out.pavements[1].surface, surface::ASPHALT);
        assert_eq!(report.converted.get("apron pieces"), Some(&3));
        let (raw, _) = run(&ap, false);
        assert_eq!(raw.pavements.len(), 3, "without union every piece stands");
    }

    #[test]
    fn distant_pieces_are_not_merged_across_tiles() {
        let ap = Airport {
            datum: LatLon::new(25.25, 55.36),
            aprons: vec![
                square(25.250, 55.360, 0.0005, Surface::Asphalt),
                square(25.260, 55.370, 0.0005, Surface::Asphalt),
            ],
            ..Default::default()
        };
        let (out, _) = run(&ap, true);
        assert_eq!(out.pavements.len(), 2);
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
        let (out, report) = run(&ap, false);
        assert_eq!(out.pavements.len(), 4, "without union, one polygon per apron");
        let order: Vec<u8> = out.pavements.iter().map(|p| p.surface).collect();
        assert_eq!(order, vec![surface::CONCRETE, surface::ASPHALT, surface::ASPHALT, surface::GRASS]);
        assert!(out.pavements.iter().all(|p| p.rings.len() == 1), "no holes");
        assert_eq!(report.converted.get("apron polygons"), Some(&4));
        let (merged, _) = run(&ap, true);
        assert_eq!(merged.pavements.len(), 3, "the overlapping asphalt pair merges");
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
