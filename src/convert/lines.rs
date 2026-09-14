//! Painted lines, hold-short bars and light strings.
//!
//! MSFS 2024 airports paint nearly all of their markings as explicit PaintedLine
//! records, which map straight onto apt.dat linear features. Older airports
//! instead derive markings from taxi path flags; those are generated only when
//! the package has no painted lines, so that nothing is painted twice.

use crate::geo::poly::offset_polyline;
use crate::geo::{LatLon, Plane};
use crate::materials::{classify_line_name, LineColour};
use crate::model::{Airport, EdgeLine, PaintedLine, PaintedLineKind, PathKind};
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
    pub const RED: u8 = 30;
    pub const RED_BROKEN: u8 = 31;
    pub const RED_WIDE: u8 = 32;
    pub const CENTRE_BORDERED: u8 = 51;
    pub const BOUNDARY_BORDERED: u8 = 52;
    pub const EDGE_BORDERED: u8 = 53;
    pub const LIGHT_CENTRE: u8 = 101;
    pub const LIGHT_EDGE: u8 = 102;
    pub const LIGHT_HOLD: u8 = 103;
    pub const LIGHT_ILS_HOLD: u8 = 105;
    pub const LIGHT_RED: u8 = 106;
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
        PaintedLineKind::SlimRed => code::RED,
        PaintedLineKind::WideRed => code::RED_WIDE,
        PaintedLineKind::Other(_) => return None,
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
    build_lines(ap, plane, out, report);
    let dark = drop_stacked_lights(&mut out.lines, plane);
    if dark > 0 {
        report.dropped("light segments lying on another light string", dark);
    }
}

fn build_lines(ap: &Airport, plane: &Plane, out: &mut AptAirport, report: &mut Report) {
    for l in &ap.painted_lines {
        match line_code_for(l) {
            Some(line) => {
                // Hold-short bars are directional in both simulators: MSFS
                // encodes the flip in the style, X-Plane in the node order.
                let mut pts = l.vertices.clone();
                if l.kind == PaintedLineKind::HoldShortBackward || l.kind == PaintedLineKind::NonMovementBack {
                    pts.reverse();
                }
                if let Some(f) = feature(&format!("{:?}", l.kind), &pts, line, light_for(l, line)) {
                    out.lines.push(f);
                    report.converted("painted lines", 1);
                }
            }
            None => report.dropped(&format!("painted lines with no X-Plane style ({:?})", l.kind), 1),
        }
    }

    for s in &ap.apron_edge_lights {
        let name = if s.name.is_empty() { "Apron edge lights" } else { &s.name };
        if let Some(f) = feature(name, &s.points, 0, light_code_for_preset(&s.name)) {
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

/// Light strings drawn on top of each other flicker: two rows of lights a few
/// centimetres apart, and temporal anti-aliasing picks a different one each
/// frame. MSFS sceneries stack them freely (O'Hare lays its amber lead-off
/// strings over the green centrelines), so where a lit segment runs along one
/// that is already lit, its lights are switched off. Stop bars win over hold
/// lights, then lead-off, edge and centre lights. Returns the segments darkened.
fn drop_stacked_lights(lines: &mut [LinearFeature], plane: &Plane) -> usize {
    // Close enough to read as the same row of lights, in metres.
    const NEAR: f64 = 1.5;
    const CELL: f64 = 10.0;
    type Seg = ((f64, f64), (f64, f64));

    fn rank(light: u8) -> u8 {
        match light {
            code::LIGHT_RED => 0,
            code::LIGHT_HOLD | 104 => 1,
            code::LIGHT_ILS_HOLD | 108 => 2,
            code::LIGHT_EDGE => 3,
            code::LIGHT_CENTRE | 107 => 4,
            _ => 5,
        }
    }
    fn cell(x: f64) -> i64 {
        (x / CELL).floor() as i64
    }
    fn dist(p: (f64, f64), (a, b): Seg) -> f64 {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
    }
    let parallel = |(a, b): Seg, (c, d): Seg| {
        let (ux, uy, vx, vy) = (b.0 - a.0, b.1 - a.1, d.0 - c.0, d.1 - c.1);
        let l = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
        l > 0.0 && (ux * vx + uy * vy).abs() / l > 0.87 // within 30 degrees
    };

    let mut grid: std::collections::HashMap<(i64, i64), Vec<Seg>> = std::collections::HashMap::new();
    let first_light = |f: &LinearFeature| f.nodes.iter().map(|n| n.light).find(|&l| l != 0);
    let mut order: Vec<usize> = (0..lines.len()).filter(|&i| first_light(&lines[i]).is_some()).collect();
    order.sort_by_key(|&i| rank(first_light(&lines[i]).unwrap_or(0)));

    let mut dark = 0;
    for fi in order {
        let f = &lines[fi];
        let n = f.nodes.len();
        let segments = if f.closed { n } else { n.saturating_sub(1) };
        let xy: Vec<(f64, f64)> = f.nodes.iter().map(|p| plane.to_xy(LatLon::new(p.lat, p.lon))).collect();
        let (mut keep, mut off) = (Vec::new(), Vec::new());
        for i in 0..segments {
            if f.nodes[i].light == 0 {
                continue;
            }
            let seg = (xy[i], xy[(i + 1) % n]);
            let len = dist(seg.1, (seg.0, seg.0));
            let samples = ((len / 2.0).ceil() as usize).max(1);
            let near = (0..=samples)
                .filter(|&k| {
                    let t = k as f64 / samples as f64;
                    let p = (seg.0 .0 + (seg.1 .0 - seg.0 .0) * t, seg.0 .1 + (seg.1 .1 - seg.0 .1) * t);
                    grid.get(&(cell(p.0), cell(p.1)))
                        .is_some_and(|v| v.iter().any(|&o| dist(p, o) < NEAR && parallel(seg, o)))
                })
                .count();
            if near * 5 >= (samples + 1) * 4 {
                off.push(i);
            } else {
                keep.push(seg);
            }
        }
        dark += off.len();
        for i in off {
            lines[fi].nodes[i].light = 0;
        }
        for seg in keep {
            let (x0, x1) = (seg.0 .0.min(seg.1 .0) - NEAR, seg.0 .0.max(seg.1 .0) + NEAR);
            let (y0, y1) = (seg.0 .1.min(seg.1 .1) - NEAR, seg.0 .1.max(seg.1 .1) + NEAR);
            for cx in cell(x0)..=cell(x1) {
                for cy in cell(y0)..=cell(y1) {
                    grid.entry((cx, cy)).or_default().push(seg);
                }
            }
        }
    }
    dark
}

/// X-Plane light string for an MSFS light preset. Sceneries put every kind of
/// taxiway light in the "apron edge lights" record and name the preset for
/// its colour and use, so the name decides the colour; unnamed strings are
/// the SDK's blue apron edge lights.
fn light_code_for_preset(name: &str) -> u8 {
    let lower = name.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |prefixes: &[&str]| words.iter().any(|w| prefixes.iter().any(|p| w.starts_with(p)));
    if has(&["blue"]) {
        code::LIGHT_EDGE
    } else if has(&["red", "stop"]) {
        code::LIGHT_RED
    } else if has(&["ils"]) {
        code::LIGHT_ILS_HOLD
    } else if has(&["hold"]) {
        code::LIGHT_HOLD
    } else if has(&["orange", "amber", "yellow", "exit", "leadoff"]) {
        // Lead-off lights alternate green and amber, like X-Plane's ILS string.
        code::LIGHT_ILS_HOLD
    } else if has(&["green", "cent", "taxi"]) {
        code::LIGHT_CENTRE
    } else {
        code::LIGHT_EDGE
    }
}

/// The light string that goes with a lighted painted line.
fn light_for(l: &PaintedLine, line: u8) -> u8 {
    if !l.lit {
        return 0;
    }
    match line {
        code::RUNWAY_HOLD | code::OTHER_HOLD => code::LIGHT_HOLD,
        code::ILS_HOLD => code::LIGHT_ILS_HOLD,
        code::EDGE | code::EDGE_BORDERED | code::WIDE_DOUBLE_BROKEN | code::BOUNDARY | code::BOUNDARY_BORDERED => {
            code::LIGHT_EDGE
        }
        _ => code::LIGHT_CENTRE,
    }
}

/// The line code for a painted line: its material name when the library
/// knows it (iniBuilds names say colour and dash pattern outright), otherwise
/// its MSFS line type.
fn line_code_for(l: &PaintedLine) -> Option<u8> {
    if let Some(name) = &l.material_name {
        let look = classify_line_name(name);
        if look.skip {
            return None;
        }
        if look.hold_short {
            return Some(code::RUNWAY_HOLD);
        }
        let bordered = |plain, with_border| Some(if look.bordered { with_border } else { plain });
        match (look.colour, look.dashed) {
            (LineColour::Yellow, _) if look.edge => return bordered(code::EDGE, code::EDGE_BORDERED),
            (LineColour::Yellow, false) => return bordered(code::CENTRE, code::CENTRE_BORDERED),
            (LineColour::Yellow, true) => return bordered(code::BOUNDARY, code::BOUNDARY_BORDERED),
            (LineColour::White, false) => return Some(code::WHITE_SOLID),
            (LineColour::White, true) => return Some(code::WHITE_BROKEN),
            (LineColour::Red, false) => return Some(code::RED),
            (LineColour::Red, true) => return Some(code::RED_BROKEN),
            (LineColour::Unknown, _) => {}
        }
    }
    painted_line_code(l.kind)
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
        assert_eq!(painted_line_code(PaintedLineKind::WideRed), Some(32));
        assert_eq!(painted_line_code(PaintedLineKind::SlimRed), Some(30));
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
                    ..Default::default()
                },
                PaintedLine {
                    kind: PaintedLineKind::EdgeSolid,
                    vertices: loop_pts,
                    ..Default::default()
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
    fn material_names_override_the_line_type() {
        let pts = vec![LatLon::new(25.0, 55.0), LatLon::new(25.001, 55.0)];
        let line = |name: &str| PaintedLine {
            kind: PaintedLineKind::Default,
            vertices: pts.clone(),
            material_name: Some(name.to_string()),
            ..Default::default()
        };
        let ap = Airport {
            datum: LatLon::new(25.0, 55.0),
            painted_lines: vec![
                line("INI_Lines_Dashed_White"),
                line("INI_CenterLine_Black"),
                line("INI_Yellow_Lines"),
                line("INI_Edge_Line_Black"),
                line("INI_Seam"),
            ],
            ..Default::default()
        };
        let out = run(&ap);
        assert_eq!(out.lines.len(), 4, "the seam is dropped");
        assert_eq!(out.lines[0].nodes[0].line, 22);
        assert_eq!(out.lines[1].nodes[0].line, 51, "yellow centreline with a black border");
        assert_eq!(out.lines[2].nodes[0].line, 1);
        assert_eq!(out.lines[3].nodes[0].line, 53, "yellow edge line with a black border");
    }

    #[test]
    fn lighted_lines_get_matching_light_strings() {
        // Side by side, 100 m apart, so no string lies on another.
        let line = |kind, lit, lon: f64| PaintedLine {
            kind,
            lit,
            vertices: vec![LatLon::new(25.0, lon), LatLon::new(25.001, lon)],
            ..Default::default()
        };
        let ap = Airport {
            datum: LatLon::new(25.0, 55.0),
            painted_lines: vec![
                line(PaintedLineKind::HoldShortForward, true, 55.0),
                line(PaintedLineKind::Default, true, 55.001),
                line(PaintedLineKind::EdgeSolid, false, 55.002),
            ],
            ..Default::default()
        };
        let out = run(&ap);
        assert_eq!(out.lines[0].nodes[0].light, 103);
        assert_eq!(out.lines[1].nodes[0].light, 101);
        assert_eq!(out.lines[2].nodes[0].light, 0);
    }

    #[test]
    fn light_presets_keep_their_colour() {
        for (name, want) in [
            ("ini-green-centre-custom", 101),
            ("DEFAULT_TAXI_CENTER", 101),
            ("Center Lights REV", 101),
            ("ini-green-edge", 101),
            ("ini-blue-edge", 102),
            ("Edge Light Blue", 102),
            ("", 102),
            ("Holdshort Lights", 103),
            ("ini-hold-short-bar REV", 103),
            ("ILS", 105),
            ("Exit Orange REV", 105),
            ("ini-orange-centre-custom", 105),
            ("Stop Bar Lights", 106),
            ("taxi light red_edge", 106),
        ] {
            assert_eq!(light_code_for_preset(name), want, "{name}");
        }
    }

    #[test]
    fn light_strings_use_their_preset_colour() {
        let ap = Airport {
            datum: LatLon::new(25.0, 55.0),
            apron_edge_lights: vec![LightString {
                name: "ini-green-centre-custom".into(),
                points: vec![LatLon::new(25.0, 55.0), LatLon::new(25.001, 55.0)],
            }],
            ..Default::default()
        };
        let out = run(&ap);
        assert_eq!(out.lines[0].nodes[0].light, 101);
        assert_eq!(out.lines[0].name, "ini-green-centre-custom");
    }

    #[test]
    fn stacked_light_strings_are_lit_once() {
        let along = |name: &str| LightString {
            name: name.into(),
            points: vec![LatLon::new(25.0, 55.0), LatLon::new(25.0, 55.001)],
        };
        let ap = Airport {
            datum: LatLon::new(25.0, 55.0),
            apron_edge_lights: vec![
                along("ini-green-centre-custom"),
                along("ini-orange-centre-custom"),
                along("ini-green-centre-custom"),
                // Crosses the others at right angles, so it keeps its lights.
                LightString {
                    name: "Holdshort Lights".into(),
                    points: vec![LatLon::new(24.9999, 55.0005), LatLon::new(25.0001, 55.0005)],
                },
            ],
            ..Default::default()
        };
        let mut report = Report::default();
        let mut out = AptAirport::default();
        build(&ap, &Plane::new(ap.datum), &mut out, &mut report);
        assert_eq!(out.lines[0].nodes[0].light, 0, "green under the lead-off string goes dark");
        assert_eq!(out.lines[1].nodes[0].light, 105);
        assert_eq!(out.lines[2].nodes[0].light, 0, "a second copy goes dark too");
        assert_eq!(out.lines[3].nodes[0].light, 103);
    }

    #[test]
    fn red_lines_are_drawn_red() {
        let line = |kind, name: Option<&str>, lon: f64| PaintedLine {
            kind,
            vertices: vec![LatLon::new(25.0, lon), LatLon::new(25.001, lon)],
            material_name: name.map(str::to_string),
            ..Default::default()
        };
        let ap = Airport {
            datum: LatLon::new(25.0, 55.0),
            painted_lines: vec![
                line(PaintedLineKind::SlimRed, None, 55.0),
                line(PaintedLineKind::Default, Some("INI_Red_Lines"), 55.001),
                line(PaintedLineKind::WideRed, None, 55.002),
            ],
            ..Default::default()
        };
        let codes: Vec<u8> = run(&ap).lines.iter().map(|l| l.nodes[0].line).collect();
        assert_eq!(codes, vec![30, 30, 32]);
    }

    #[test]
    fn backward_hold_short_is_reversed() {
        let pts = vec![LatLon::new(25.0, 55.0), LatLon::new(25.0, 55.001)];
        let ap = Airport {
            datum: LatLon::new(25.0, 55.0),
            painted_lines: vec![PaintedLine {
                kind: PaintedLineKind::HoldShortBackward,
                vertices: pts,
                ..Default::default()
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
            ..Default::default()
        });
        assert_eq!(run(&ap).lines.len(), 1);
    }
}
