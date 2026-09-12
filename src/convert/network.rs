//! The ATC taxi route network (rows 1200-1206).
//!
//! MSFS taxi paths are already a graph, so the nodes and edges carry over
//! directly. What X-Plane needs on top is "active zones": which taxi edges are
//! close enough to a runway that using them needs a clearance. MSFS marks the
//! boundary with hold-short nodes, so the zone of each runway is the part of the
//! network reachable from it without crossing a hold-short node, limited to a
//! band around the runway so an airport with missing hold nodes does not mark
//! every taxiway as hot.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::geo::{LatLon, Plane};
use crate::model::{Airport, PathKind};
use crate::xplane::apt::{ActiveZone, NetEdge, NetNode, Network, TruckEdge};

use super::runways::runway_end_positions;
use super::tables::width_class_from_taxiway;
use super::Report;

/// How far from a runway centreline a taxi edge can be and still be in its zone.
const ZONE_BAND_M: f64 = 150.0;

struct RunwayZone {
    /// The name X-Plane uses for the runway as a whole, e.g. `12L/30R`.
    pair: String,
    ends: [String; 2],
    a: (f64, f64),
    b: (f64, f64),
}

fn dist_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-9 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

pub fn build(ap: &Airport, plane: &Plane, report: &mut Report) -> Option<Network> {
    let positions: HashMap<usize, LatLon> = ap.taxi_nodes.iter().map(|n| (n.index, n.pos)).collect();
    let kinds: HashMap<usize, crate::model::NodeKind> = ap.taxi_nodes.iter().map(|n| (n.index, n.kind)).collect();

    let runways: Vec<RunwayZone> = ap
        .runways
        .iter()
        .filter(|r| !r.surface.is_water())
        .map(|r| {
            let (p, s) = runway_end_positions(r);
            RunwayZone {
                pair: format!("{}/{}", r.ends[0].name, r.ends[1].name),
                ends: [r.ends[0].name.clone(), r.ends[1].name.clone()],
                a: plane.to_xy(p),
                b: plane.to_xy(s),
            }
        })
        .collect();
    let runway_by_end: HashMap<&str, usize> = runways
        .iter()
        .enumerate()
        .flat_map(|(i, r)| r.ends.iter().map(move |e| (e.as_str(), i)))
        .collect();

    // Compact node ids: X-Plane wants them dense and zero-based.
    let mut ids: HashMap<usize, usize> = HashMap::new();
    let mut nodes: Vec<NetNode> = Vec::new();
    let mut id_of = |index: usize, nodes: &mut Vec<NetNode>| -> Option<usize> {
        if let Some(&id) = ids.get(&index) {
            return Some(id);
        }
        let pos = positions.get(&index)?;
        let id = nodes.len();
        nodes.push(NetNode {
            lat: pos.lat,
            lon: pos.lon,
            usage: "both".into(),
            id,
            name: format!("n{id}"),
        });
        ids.insert(index, id);
        Some(id)
    };

    let mut edges: Vec<NetEdge> = Vec::new();
    let mut edge_runway: Vec<Option<usize>> = Vec::new();
    let mut truck_edges: Vec<TruckEdge> = Vec::new();
    let mut seen: HashSet<(usize, usize, bool)> = HashSet::new();

    for p in &ap.taxi_paths {
        let aircraft = p.kind.is_aircraft_route();
        let truck = p.kind.is_ground_vehicle();
        if !aircraft && !truck {
            continue;
        }
        let (Some(a), Some(b)) = (id_of(p.start, &mut nodes), id_of(p.end, &mut nodes)) else {
            report.dropped("taxi paths with missing nodes", 1);
            continue;
        };
        let (pa, pb) = (&nodes[a], &nodes[b]);
        let (la, lb) = (LatLon::new(pa.lat, pa.lon), LatLon::new(pb.lat, pb.lon));
        let (xa, xb) = (plane.to_xy(la), plane.to_xy(lb));
        let length = ((xa.0 - xb.0).powi(2) + (xa.1 - xb.1).powi(2)).sqrt();
        if a == b || length < 0.5 {
            report.dropped("zero-length taxi paths", 1);
            continue;
        }
        let key = (a.min(b), a.max(b), truck);
        if !seen.insert(key) {
            report.dropped("duplicate taxi paths", 1);
            continue;
        }
        if truck {
            truck_edges.push(TruckEdge {
                from: a,
                to: b,
                oneway: false,
                name: p.name.clone(),
                shape: vec![],
            });
            continue;
        }
        let (kind, name, rw) = if p.kind == PathKind::Runway {
            let rw = runway_by_end.get(p.name.as_str()).copied();
            let name = rw.map(|i| runways[i].pair.clone()).unwrap_or_else(|| p.name.clone());
            ("runway".to_string(), name, rw)
        } else {
            (
                format!("taxiway_{}", width_class_from_taxiway(p.width_m)),
                p.name.clone(),
                None,
            )
        };
        edges.push(NetEdge {
            from: a,
            to: b,
            oneway: false,
            kind,
            name,
            shape: vec![],
            active_zones: vec![],
        });
        edge_runway.push(rw);
    }

    if edges.is_empty() && truck_edges.is_empty() {
        return None;
    }

    // Active zones.
    let index_of_id: HashMap<usize, usize> = ids.iter().map(|(&idx, &id)| (id, idx)).collect();
    let mut adjacency: HashMap<usize, Vec<usize>> = HashMap::new();
    for (e, edge) in edges.iter().enumerate() {
        adjacency.entry(edge.from).or_default().push(e);
        adjacency.entry(edge.to).or_default().push(e);
    }
    let mut zones: Vec<Vec<(String, usize)>> = vec![Vec::new(); edges.len()];
    for (ri, rw) in runways.iter().enumerate() {
        let mut visited_edges: HashSet<usize> = HashSet::new();
        let mut queue: VecDeque<usize> = VecDeque::new();
        for (e, owner) in edge_runway.iter().enumerate() {
            if *owner == Some(ri) {
                zones[e].push(("departure".into(), ri));
                zones[e].push(("arrival".into(), ri));
                queue.push_back(edges[e].from);
                queue.push_back(edges[e].to);
            }
        }
        let mut visited_nodes: HashSet<usize> = queue.iter().copied().collect();
        while let Some(n) = queue.pop_front() {
            for &e in adjacency.get(&n).map(Vec::as_slice).unwrap_or(&[]) {
                if edge_runway[e].is_some() || !visited_edges.insert(e) {
                    continue;
                }
                let other = if edges[e].from == n { edges[e].to } else { edges[e].from };
                let op = LatLon::new(nodes[other].lat, nodes[other].lon);
                if dist_to_segment(plane.to_xy(op), rw.a, rw.b) > ZONE_BAND_M {
                    continue;
                }
                zones[e].push(("departure".into(), ri));
                zones[e].push(("arrival".into(), ri));
                let hold = index_of_id.get(&other).and_then(|i| kinds.get(i)).copied();
                if let Some(k) = hold.filter(|k| k.is_hold_short()) {
                    if k.is_ils() {
                        zones[e].push(("ils".into(), ri));
                    }
                    continue; // do not expand past a hold-short node
                }
                if visited_nodes.insert(other) {
                    queue.push_back(other);
                }
            }
        }
    }
    let mut hot = 0;
    for (e, list) in zones.into_iter().enumerate() {
        let mut by_phase: Vec<ActiveZone> = Vec::new();
        for phase in ["departure", "arrival", "ils"] {
            let mut rws: Vec<String> = list
                .iter()
                .filter(|(p, _)| p == phase)
                .flat_map(|(_, ri)| runways[*ri].ends.iter().cloned())
                .collect();
            rws.sort();
            rws.dedup();
            if !rws.is_empty() {
                by_phase.push(ActiveZone {
                    phase: phase.into(),
                    runways: rws,
                });
            }
        }
        if !by_phase.is_empty() {
            hot += 1;
        }
        edges[e].active_zones = by_phase;
    }

    report.converted("taxi network nodes", nodes.len());
    report.converted("taxi network edges", edges.len());
    report.converted("service road edges", truck_edges.len());
    report.converted("edges in runway active zones", hot);
    Some(Network {
        name: ap.icao.clone(),
        nodes,
        edges,
        truck_edges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    /// A runway along the equator-ish x axis with a taxiway leaving it: runway
    /// node 0-1, then 1 -> 2 (hold short) -> 3 further away.
    fn airport(hold_kind: NodeKind) -> Airport {
        let datum = LatLon::new(25.0, 55.0);
        let n = |index, lat, lon, kind| TaxiNode {
            index,
            pos: LatLon::new(lat, lon),
            kind,
            reverse: false,
        };
        let path = |start, end, kind, name: &str| TaxiPath {
            start,
            end,
            kind,
            name: name.into(),
            width_m: 23.0,
            ..Default::default()
        };
        Airport {
            icao: "TEST".into(),
            datum,
            runways: vec![Runway {
                centre: LatLon::new(25.0, 55.01),
                heading_true: 90.0,
                length_m: 2000.0,
                width_m: 45.0,
                surface: Surface::Asphalt,
                ends: [
                    RunwayEnd {
                        name: "09".into(),
                        ..Default::default()
                    },
                    RunwayEnd {
                        name: "27".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            taxi_nodes: vec![
                n(0, 25.0, 55.0, NodeKind::Normal),
                n(1, 25.0, 55.01, NodeKind::Normal),
                n(2, 25.0006, 55.01, hold_kind),
                n(3, 25.002, 55.01, NodeKind::Normal),
                n(4, 25.003, 55.01, NodeKind::Normal),
            ],
            taxi_paths: vec![
                path(0, 1, PathKind::Runway, "09"),
                path(1, 2, PathKind::Taxi, "A"),
                path(2, 3, PathKind::Taxi, "A"),
                path(3, 4, PathKind::Vehicle, "service"),
                path(3, 3, PathKind::Taxi, "loop"),
            ],
            ..Default::default()
        }
    }

    fn net(ap: &Airport) -> (Network, Report) {
        let mut report = Report::default();
        let n = build(ap, &Plane::new(ap.datum), &mut report).unwrap();
        (n, report)
    }

    #[test]
    fn nodes_are_compact_and_edges_typed() {
        let (n, report) = net(&airport(NodeKind::HoldShort));
        assert_eq!(n.nodes.iter().map(|x| x.id).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        assert_eq!(n.edges.len(), 3);
        assert_eq!(n.edges[0].kind, "runway");
        assert_eq!(n.edges[0].name, "09/27", "runway edges carry the full runway name");
        assert_eq!(n.edges[1].kind, "taxiway_E");
        assert_eq!(n.truck_edges.len(), 1);
        assert_eq!(report.dropped.get("zero-length taxi paths"), Some(&1));
    }

    #[test]
    fn active_zone_stops_at_the_hold_short_node() {
        let (n, _) = net(&airport(NodeKind::HoldShort));
        assert!(!n.edges[0].active_zones.is_empty(), "the runway itself is hot");
        assert_eq!(n.edges[1].active_zones.len(), 2, "runway to hold short is hot");
        assert_eq!(
            n.edges[1].active_zones[0].runways,
            vec!["09".to_string(), "27".to_string()]
        );
        assert!(n.edges[2].active_zones.is_empty(), "beyond the hold short is not");
    }

    #[test]
    fn ils_hold_adds_an_ils_zone() {
        let (n, _) = net(&airport(NodeKind::IlsHoldShort));
        assert!(n.edges[1].active_zones.iter().any(|z| z.phase == "ils"));
    }

    #[test]
    fn zones_do_not_flood_without_hold_nodes() {
        let (n, _) = net(&airport(NodeKind::Normal));
        // Node 3 is ~220 m from the runway, outside the band.
        assert!(n.edges[2].active_zones.is_empty());
    }

    #[test]
    fn output_validates() {
        let mut ap = airport(NodeKind::HoldShort);
        ap.name = "Test".into();
        let (out, _) = crate::convert::convert(&ap, &crate::convert::Options::default());
        let text = crate::xplane::write(&crate::xplane::Apt { airports: vec![out] });
        crate::xplane::validate(&text).unwrap_or_else(|e| panic!("{e:#?}"));
    }
}
