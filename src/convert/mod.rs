//! Turning the neutral airport model into apt.dat rows.
//!
//! Each concern lives in its own module and appends to one [`AptAirport`]. The
//! [`Report`] collects what was converted, what was approximated and what had
//! no X-Plane equivalent, so that nothing is lost silently.

pub mod lines;
pub mod network;
pub mod pavement;
pub mod ramps;
pub mod runways;
pub mod signs;
pub mod tables;

use std::collections::BTreeMap;

use crate::geo::Plane;
use crate::model::Airport;
use crate::xplane::apt::{self, row, AptAirport};

/// What to generate.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Union overlapping pavement into clean outlines.
    pub union: bool,
    /// Generate the ATC taxi route network.
    pub network: bool,
    /// Generate painted lines and light strings.
    pub lines: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            union: true,
            network: true,
            lines: true,
        }
    }
}

/// What happened to one airport.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Report {
    pub icao: String,
    pub name: String,
    pub package: String,
    pub sim: String,
    /// Features written, by kind.
    pub converted: BTreeMap<String, usize>,
    /// Features with no X-Plane equivalent or that failed a sanity check.
    pub dropped: BTreeMap<String, usize>,
    pub warnings: Vec<String>,
}

impl Report {
    pub fn converted(&mut self, what: &str, n: usize) {
        if n > 0 {
            *self.converted.entry(what.to_string()).or_default() += n;
        }
    }

    pub fn dropped(&mut self, what: &str, n: usize) {
        if n > 0 {
            *self.dropped.entry(what.to_string()).or_default() += n;
        }
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }
}

/// Metres to feet, as apt.dat expresses elevations.
pub fn feet(m: f64) -> f64 {
    m * 3.280_839_9
}

fn looks_like_icao(s: &str) -> bool {
    s.len() == 4 && s.chars().all(|c| c.is_ascii_uppercase())
}

/// Convert one airport.
/// Convex hull of `pts` (plane metres), pushed out by `margin` metres from
/// its centre, counter-clockwise. Fewer than three distinct points give none.
fn boundary_ring(pts: &[(f64, f64)], margin: f64) -> Vec<(f64, f64)> {
    let mut p: Vec<(f64, f64)> = pts.iter().copied().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
    p.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    p.dedup();
    if p.len() < 3 {
        return Vec::new();
    }
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<(f64, f64)> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<(f64, f64)> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    if lower.len() < 3 {
        return Vec::new();
    }
    let n = lower.len() as f64;
    let c = (lower.iter().map(|q| q.0).sum::<f64>() / n, lower.iter().map(|q| q.1).sum::<f64>() / n);
    lower
        .into_iter()
        .map(|q| {
            let (dx, dy) = (q.0 - c.0, q.1 - c.1);
            let d = dx.hypot(dy).max(1e-9);
            (q.0 + dx / d * margin, q.1 + dy / d * margin)
        })
        .collect()
}

pub fn convert(ap: &Airport, opts: &Options) -> (AptAirport, Report) {
    let mut report = Report {
        icao: ap.icao.clone(),
        name: ap.name.clone(),
        package: ap.source.package.clone(),
        sim: ap.source.sim.label().to_string(),
        warnings: ap.warnings.clone(),
        ..Default::default()
    };
    let plane = Plane::new(ap.datum);

    let all_water = !ap.runways.is_empty() && ap.runways.iter().all(|r| r.surface.is_water());
    let kind = if ap.runways.is_empty() && !ap.helipads.is_empty() {
        row::HELIPORT
    } else if all_water {
        row::SEAPLANE_BASE
    } else {
        row::LAND_AIRPORT
    };

    let mut out = AptAirport {
        kind,
        elevation_ft: feet(ap.elevation_m).round() as i32,
        icao: ap.icao.clone(),
        name: if ap.name.is_empty() {
            ap.icao.clone()
        } else {
            ap.name.clone()
        },
        ..Default::default()
    };

    if looks_like_icao(&ap.icao) {
        out.metadata.push(("icao_code".into(), ap.icao.clone()));
    }
    out.metadata.push(("datum_lat".into(), format!("{:.6}", ap.datum.lat)));
    out.metadata.push(("datum_lon".into(), format!("{:.6}", ap.datum.lon)));
    for (key, value) in [("city", &ap.city), ("state", &ap.state), ("country", &ap.country)] {
        if let Some(v) = value.as_ref().filter(|v| !v.is_empty()) {
            out.metadata.push((key.into(), v.clone()));
        }
    }
    if let Some(region) = ap.region.as_ref().filter(|r| r.len() == 2) {
        out.metadata.push(("region_code".into(), region.clone()));
    }

    runways::build(ap, &mut out, &mut report);
    pavement::build(ap, &plane, opts, &mut out, &mut report);

    // An airport boundary around everything drawn, with flattening on, so
    // X-Plane levels its terrain to the field elevation the way MSFS does.
    // Without it pavement, buildings and vehicles each follow uneven ground.
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for p in &out.pavements {
        for n in p.rings.iter().take(1).flatten() {
            pts.push(plane.to_xy(crate::geo::LatLon::new(n.lat, n.lon)));
        }
    }
    for r in &out.runways {
        for e in &r.ends {
            pts.push(plane.to_xy(crate::geo::LatLon::new(e.lat, e.lon)));
        }
    }
    for h in &out.helipads {
        pts.push(plane.to_xy(crate::geo::LatLon::new(h.lat, h.lon)));
    }
    let ring = boundary_ring(&pts, 60.0);
    if ring.len() >= 3 {
        out.boundary = ring
            .iter()
            .map(|&(x, y)| {
                let p = plane.to_latlon(x, y);
                apt::Node::at(p.lat, p.lon)
            })
            .collect();
        out.metadata.push(("flatten".into(), "1".into()));
        report.converted("airport boundary", 1);
    }
    if opts.lines {
        lines::build(ap, &plane, &mut out, &mut report);
    }

    if let Some(t) = &ap.tower {
        let height_ft = feet(t.elevation_m - ap.elevation_m).max(10.0) as f32;
        out.tower = Some(apt::Tower {
            lat: t.pos.lat,
            lon: t.pos.lon,
            height_ft,
            // The package's own tower model comes across with the other buildings.
            draw: false,
            name: "Tower".into(),
        });
        report.converted("tower", 1);
    }
    for w in &ap.windsocks {
        out.windsocks.push(apt::Windsock {
            lat: w.pos.lat,
            lon: w.pos.lon,
            lit: w.lit,
            name: "Windsock".into(),
        });
    }
    report.converted("windsocks", ap.windsocks.len());
    for b in &ap.beacons {
        out.beacons.push(apt::Beacon {
            lat: b.pos.lat,
            lon: b.pos.lon,
            kind: if kind == row::HELIPORT {
                3
            } else if kind == row::SEAPLANE_BASE {
                2
            } else {
                1
            },
            name: "Beacon".into(),
        });
    }
    report.converted("beacons", ap.beacons.len());

    signs::build(ap, &mut out, &mut report);

    for c in &ap.coms {
        match tables::com_code(c.kind) {
            Some(code) if (108_000..=137_000).contains(&c.freq_khz) => {
                out.frequencies.push(apt::Frequency {
                    code,
                    khz: c.freq_khz,
                    name: if c.name.is_empty() {
                        format!("{} {:?}", ap.icao, c.kind).to_uppercase()
                    } else {
                        c.name.clone()
                    },
                });
                report.converted("frequencies", 1);
            }
            _ => report.dropped("frequencies with no X-Plane type", 1),
        }
    }

    if opts.network {
        out.network = network::build(ap, &plane, &mut report);
    }
    ramps::build(ap, &plane, &mut out, &mut report);

    report.dropped("start positions (X-Plane derives these)", ap.starts.len());
    (out, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_is_a_padded_convex_hull() {
        let pts = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (50.0, 50.0), (50.0, 50.0)];
        let r = boundary_ring(&pts, 10.0);
        assert_eq!(r.len(), 4, "interior points are dropped");
        assert!(crate::geo::poly::signed_area(&r) > 100.0 * 100.0, "padded outward, counter-clockwise");
        assert!(boundary_ring(&[(0.0, 0.0), (1.0, 1.0)], 10.0).is_empty());
    }
    use crate::geo::LatLon;
    use crate::model::*;

    pub(crate) fn tiny_airport() -> Airport {
        let end = |name: &str| RunwayEnd {
            name: name.into(),
            takeoff: true,
            landing: true,
            ..Default::default()
        };
        Airport {
            icao: "OMDB".into(),
            name: "Dubai Intl".into(),
            datum: LatLon::new(25.2528, 55.3644),
            elevation_m: 19.0,
            runways: vec![Runway {
                centre: LatLon::new(25.2528, 55.3644),
                heading_true: 120.0,
                length_m: 4000.0,
                width_m: 60.0,
                surface: Surface::Asphalt,
                ends: [end("12L"), end("30R")],
                edge_lights: LightLevel::High,
                ..Default::default()
            }],
            coms: vec![Com {
                kind: ComKind::Tower,
                freq_khz: 118_750,
                name: "DUBAI TOWER".into(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn airport_kind_follows_the_facilities() {
        let (out, _) = convert(&tiny_airport(), &Options::default());
        assert_eq!(out.kind, row::LAND_AIRPORT);

        let mut heli = tiny_airport();
        heli.runways.clear();
        heli.helipads.push(Helipad {
            width_m: 20.0,
            length_m: 20.0,
            ..Default::default()
        });
        assert_eq!(convert(&heli, &Options::default()).0.kind, row::HELIPORT);

        let mut sea = tiny_airport();
        sea.runways[0].surface = Surface::Water;
        assert_eq!(convert(&sea, &Options::default()).0.kind, row::SEAPLANE_BASE);
    }

    #[test]
    fn elevation_is_written_in_feet_with_metadata() {
        let (out, report) = convert(&tiny_airport(), &Options::default());
        assert_eq!(out.elevation_ft, 62);
        assert!(out.metadata.contains(&("icao_code".into(), "OMDB".into())));
        assert_eq!(out.frequencies.len(), 1);
        assert_eq!(out.frequencies[0].code, row::FREQ_TOWER);
        assert_eq!(report.converted.get("frequencies"), Some(&1));
    }

    #[test]
    fn output_of_a_small_airport_validates() {
        let mut ap = tiny_airport();
        ap.aprons.push(Apron {
            surface: Surface::Concrete,
            draw: true,
            vertices: vec![
                LatLon::new(25.250, 55.360),
                LatLon::new(25.251, 55.360),
                LatLon::new(25.251, 55.361),
                LatLon::new(25.250, 55.361),
            ],
            ..Default::default()
        });
        let (out, _) = convert(&ap, &Options::default());
        let text = crate::xplane::write(&crate::xplane::Apt { airports: vec![out] });
        crate::xplane::validate(&text).unwrap_or_else(|e| panic!("{e:#?}\n{text}"));
    }
}
