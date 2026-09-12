//! Painted lines, hold-short bars and light strings.
//!
//! MSFS 2024 airports paint nearly all of their markings as explicit PaintedLine
//! records, which map straight onto apt.dat linear features. Older airports
//! instead derive markings from taxi path flags; those are generated only when
//! the package has no painted lines, so that nothing is painted twice.

use crate::geo::poly::offset_polyline;
use crate::geo::{LatLon, Plane};
use crate::model::{Airport, EdgeLine, PaintedLineKind, PathKind};
use crate::xplane::apt::{AptAirport, LinearFeature, Node};

use super::Report;

/// X-Plane line codes used here.
mod code {
    pub const CENTRE: u8 = 1;
    pub const BOUNDARY: u8 = 2;
    pub const EDGE: u8 = 3;
    pub const RUNWAY_HOLD: u8 = 4;
    pub const OTHER_HOLD: u8 = 5;
    pub const ILS_HOLD: u8 = 6;
    pub const WIDE_DOUBLE_BROKEN: u8 = 9;
    pub const WHITE_SOLID: u8 = 20;
    pub const WHITE_BROKEN: u8 = 22;
    pub const LIGHT_CENTRE: u8 = 101;
    pub const LIGHT_EDGE: u8 = 102;
    pub const LIGHT_HOLD: u8 = 103;
    pub const LIGHT_ILS_HOLD: u8 = 105;
}

/// X-Plane line code for an MSFS painted line style, or `None` when X-Plane has
/// nothing comparable (red lines, for instance).
pub fn painted_line_code(k: PaintedLineKind) -> Option<u8> {
    Some(match k {
        PaintedLineKind::Default | PaintedLineKind::EnhancedCentre | PaintedLineKind::WideYellow => code::CENTRE,
        PaintedLineKind::HoldShortForward | PaintedLineKind::HoldShortBackward => code::RUNWAY_HOLD,
        PaintedLineKind::HoldShortTaxiway => code::OTHER_HOLD,
        PaintedLineKind::IlsHoldShort => code::ILS_HOLD,
        PaintedLineKind::EdgeSolid | PaintedLineKind::EdgeSolidOrtho => code::EDGE,
        PaintedLineKind::EdgeDashed => code::WIDE_DOUBLE_BROKEN,
        PaintedLineKind::NonMovement | PaintedLineKind::NonMovementBack => code::BOUNDARY,
        PaintedLineKind::EdgeServiceSolid | PaintedLineKind::WideWhite => code::WHITE_SOLID,
        PaintedLineKind::EdgeServiceDashed | PaintedLineKind::ServiceDashed => code::WHITE_BROKEN,
        PaintedLineKind::WideRed | PaintedLineKind::SlimRed | PaintedLineKind::Other(_) => return None,
    })
}

fn feature(name: &str, points: &[LatLon], line: u8, light: u8) -> Option<LinearFeature> {
    if points.len() < 2 {
        return None;
    }
    let closed = points.len() >= 4 && {
        let (a, b) = (points[0], points[points.len() - 1]);
        (a.lat - b.lat).abs() < 1e-9 && (a.lon - b.lon).abs() < 1e-9
    };
    let pts = if closed { &points[..points.len() - 1] } else { points };
    Some(LinearFeature {
        name: name.to_string(),
        nodes: pts
            .iter()
            .map(|p| Node {
                lat: p.lat,
                lon: p.lon,
                control: None,
                line,
                light,
            })
            .collect(),
        closed,
    })
}

pub fn build(ap: &Airport, plane: &Plane, out: &mut AptAirport, report: &mut Report) {
    for l in &ap.painted_lines {
        match painted_line_kind(l.kind) {
            Some(line) => {
                // Hold-short bars are directional in both simulators: MSFS
                // encodes the flip in the style, X-Plane in the node order.
                let mut pts = l.vertices.clone();
                if l.kind == PaintedLineKind::HoldShortBackward || l.kind == PaintedLineKind::NonMovementBack {
                    pts.reverse();
                }
                if let Some(f) = feature(&format!("{:?}", l.kind), &pts, line, 0) {
                    out.lines.push(f);
                    report.converted("painted lines", 1);
                }
            }
            None => report.dropped(&format!("painted lines with no X-Plane style ({:?})", l.kind), 1),
        }
    }

    for poly in &ap.apron_edge_lights {
        if let Some(f) = feature("Apron edge lights", poly, 0, code::LIGHT_EDGE) {
            out.lines.push(f);
            report.converted("light strings", 1);
        }
    }

    if !ap.painted_lines.is_empty() {
        return;
    }

    // No painted lines: derive markings from the taxi path flags instead.
    let nodes: std::collections::HashMap<usize, LatLon> = ap.taxi_nodes.iter().map(|n| (n.index, n.pos)).collect();
    for p in &ap.taxi_paths {
        if !matches!(p.kind, PathKind::Taxi | PathKind::Parking | PathKind::Path) {
            continue;
        }
        let (Some(&a), Some(&b)) = (nodes.get(&p.start), nodes.get(&p.end)) else {
            continue;
        };
        if p.centre_line {
            let light = if p.centre_line_lit { code::LIGHT_CENTRE } else { 0 };
            if let Some(f) = feature(&p.name, &[a, b], code::CENTRE, light) {
                out.lines.push(f);
                report.converted("taxiway centrelines", 1);
            }
        }
        let xy = [plane.to_xy(a), plane.to_xy(b)];
        for (edge, lit, sign) in [
            (p.left_edge, p.left_edge_lit, 1.0),
            (p.right_edge, p.right_edge_lit, -1.0),
        ] {
            let line = match edge {
                EdgeLine::None => 0,
                EdgeLine::Solid | EdgeLine::SolidDashed => code::EDGE,
                EdgeLine::Dashed => code::WIDE_DOUBLE_BROKEN,
            };
            if line == 0 && !lit {
                continue;
            }
            let off = offset_polyline(&xy, sign * p.width_m as f64 / 2.0);
            let pts: Vec<LatLon> = off.iter().map(|&(x, y)| plane.to_latlon(x, y)).collect();
            if let Some(f) = feature(&p.name, &pts, line, if lit { code::LIGHT_EDGE } else { 0 }) {
                out.lines.push(f);
                report.converted("taxiway edge lines", 1);
            }
        }
    }

    // Hold-short bars across the taxiway at each drawn hold node.
    for n in ap.taxi_nodes.iter().filter(|n| n.kind.draws_bar()) {
        let Some(path) = ap
            .taxi_paths
            .iter()
            .find(|p| (p.start == n.index || p.end == n.index) && p.kind != PathKind::Runway)
        else {
            continue;
        };
        let other = if path.start == n.index { path.end } else { path.start };
        let Some(&o) = nodes.get(&other) else { continue };
        let (cx, cy) = plane.to_xy(n.pos);
        let (ox, oy) = plane.to_xy(o);
        let (dx, dy) = (ox - cx, oy - cy);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 0.1 {
            continue;
        }
        let half = path.width_m.max(10.0) as f64 / 2.0;
        let (px, py) = (-dy / len * half, dx / len * half);
        let mut ends = [plane.to_latlon(cx + px, cy + py), plane.to_latlon(cx - px, cy - py)];
        if n.reverse {
            ends.reverse();
        }
        let (line, light) = if n.kind.is_ils() {
            (code::ILS_HOLD, code::LIGHT_ILS_HOLD)
        } else {
            (code::RUNWAY_HOLD, code::LIGHT_HOLD)
        };
        if let Some(f) = feature("Hold short", &ends, line, light) {
            out.lines.push(f);
            report.converted("hold-short bars", 1);
        }
    }
}

fn painted_line_kind(k: PaintedLineKind) -> Option<u8> {
    painted_line_code(k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn run(ap: &Airport) -> AptAirport {
        let mut out = AptAirport::default();
        build(ap, &Plane::new(ap.datum), &mut out, &mut Report::default());
        out
    }

    #[test]
    fn painted_line_styles_map() {
        assert_eq!(painted_line_code(PaintedLineKind::HoldShortForward), Some(4));
        assert_eq!(painted_line_code(PaintedLineKind::IlsHoldShort), Some(6));
        assert_eq!(painted_line_code(PaintedLineKind::EdgeSolid), Some(3));
        assert_eq!(painted_line_code(PaintedLineKind::ServiceDashed), Some(22));
        assert_eq!(painted_line_code(PaintedLineKind::WideRed), None);
    }

    #[test]
    fn painted_lines_become_features_and_closed_loops_close() {
        let loop_pts = vec![
            LatLon::new(25.0, 55.0),
            LatLon::new(25.001, 55.0),
            LatLon::new(25.001, 55.001),
            LatLon::new(25.0, 55.0),
        ];
        let ap = Airport {
            datum: LatLon::new(25.0, 55.0),
            painted_lines: vec![
                PaintedLine {
                    kind: PaintedLineKind::Default,
                    vertices: vec![LatLon::new(25.0, 55.0), LatLon::new(25.001, 55.0)],
                },
                PaintedLine {
                    kind: PaintedLineKind::EdgeSolid,
                    vertices: loop_pts,
                },
            ],
            ..Default::default()
        };
        let out = run(&ap);
        assert_eq!(out.lines.len(), 2);
        assert!(!out.lines[0].closed);
        assert!(out.lines[1].closed);
        assert_eq!(out.lines[1].nodes.len(), 3, "the repeated closing vertex is dropped");
        assert_eq!(out.lines[1].nodes[0].line, 3);
    }

    #[test]
    fn backward_hold_short_is_reversed() {
        let pts = vec![LatLon::new(25.0, 55.0), LatLon::new(25.0, 55.001)];
        let ap = Airport {
            datum: LatLon::new(25.0, 55.0),
            painted_lines: vec![PaintedLine {
                kind: PaintedLineKind::HoldShortBackward,
                vertices: pts,
            }],
            ..Default::default()
        };
        let out = run(&ap);
        assert!(out.lines[0].nodes[0].lon > out.lines[0].nodes[1].lon);
    }

    #[test]
    fn path_flags_are_used_only_without_painted_lines() {
        let n = |index, lat| TaxiNode {
            index,
            pos: LatLon::new(lat, 55.0),
            kind: if index == 1 {
                NodeKind::HoldShort
            } else {
                NodeKind::Normal
            },
            ..Default::default()
        };
        let mut ap = Airport {
            datum: LatLon::new(25.0, 55.0),
            taxi_nodes: vec![n(0, 25.0), n(1, 25.001)],
            taxi_paths: vec![TaxiPath {
                start: 0,
                end: 1,
                kind: PathKind::Taxi,
                name: "A".into(),
                width_m: 20.0,
                centre_line: true,
                centre_line_lit: true,
                left_edge: EdgeLine::Solid,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = run(&ap);
        assert_eq!(out.lines.len(), 3, "centreline, one edge and a hold-short bar");
        assert_eq!(out.lines[0].nodes[0].light, 101);
        ap.painted_lines.push(PaintedLine {
            kind: PaintedLineKind::Default,
            vertices: vec![LatLon::new(25.0, 55.0), LatLon::new(25.001, 55.0)],
        });
        assert_eq!(run(&ap).lines.len(), 1);
    }
}
