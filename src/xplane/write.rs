//! Rendering an [`Apt`] document as apt.dat 1200 text.
//!
//! Row order inside an airport follows the order WorldEditor writes, which is
//! also the order X-Plane's own loader is most forgiving of: header and metadata,
//! runways, pavement, linear features, point objects, radios, the ATC network,
//! then ramp starts, jetways and service vehicles.

use std::fmt::Write as _;

use super::apt::*;

const EOL: &str = "\r\n";

/// Coordinates are written with eight decimals: about a millimetre.
fn ll(lat: f64, lon: f64) -> String {
    format!("{lat:.8} {lon:.8}")
}

fn hdg(h: f32) -> f32 {
    let mut h = h % 360.0;
    if h < 0.0 {
        h += 360.0;
    }
    if h >= 359.995 {
        0.0
    } else {
        h
    }
}

/// Free-text columns cannot contain line breaks; collapse all whitespace runs.
fn text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A single-token column: no spaces allowed at all.
fn token(s: &str, fallback: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if t.is_empty() {
        fallback.to_string()
    } else {
        t
    }
}

fn node_row(out: &mut String, n: &Node, last: bool, closed: bool) {
    let code = match (n.control.is_some(), last, closed) {
        (false, false, _) => row::NODE,
        (true, false, _) => row::NODE_BEZIER,
        (false, true, true) => row::NODE_CLOSE,
        (true, true, true) => row::NODE_CLOSE_BEZIER,
        (false, true, false) => row::NODE_END,
        (true, true, false) => row::NODE_END_BEZIER,
    };
    let _ = write!(out, "{code} {}", ll(n.lat, n.lon));
    if let Some((clat, clon)) = n.control {
        let _ = write!(out, " {}", ll(clat, clon));
    }
    // End nodes of open lines take no attributes: the line ends there.
    if !(last && !closed) {
        if n.line != 0 {
            let _ = write!(out, " {}", n.line);
        }
        if n.light != 0 {
            let _ = write!(out, " {}", n.light);
        }
    }
    out.push_str(EOL);
}

fn write_ring(out: &mut String, ring: &[Node], closed: bool) {
    for (i, n) in ring.iter().enumerate() {
        node_row(out, n, i + 1 == ring.len(), closed);
    }
}

fn write_airport(out: &mut String, ap: &AptAirport) {
    let _ = write!(
        out,
        "{EOL}{} {} 0 0 {} {}{EOL}",
        ap.kind,
        ap.elevation_ft,
        token(&ap.icao, "XXXX"),
        text(&ap.name)
    );
    for (k, v) in &ap.metadata {
        let _ = write!(out, "{} {} {}{EOL}", row::METADATA, token(k, "key"), text(v));
    }

    for r in &ap.runways {
        let _ = write!(
            out,
            "{} {:.2} {} {} {:.2} {} {} {}",
            row::RUNWAY,
            r.width_m,
            r.surface,
            r.shoulder,
            r.smoothness,
            u8::from(r.centre_lights),
            r.edge_lights,
            u8::from(r.distance_signs)
        );
        for e in &r.ends {
            let _ = write!(
                out,
                " {} {} {:.2} {:.2} {} {} {} {}",
                token(&e.name, "00"),
                ll(e.lat, e.lon),
                e.displaced_m,
                e.blast_pad_m,
                e.markings,
                e.approach_lights,
                u8::from(e.touchdown_lights),
                e.reil
            );
        }
        out.push_str(EOL);
    }
    for w in &ap.water_runways {
        let _ = write!(out, "{} {:.2} {}", row::WATER_RUNWAY, w.width_m, u8::from(w.buoys));
        for (name, lat, lon) in &w.ends {
            let _ = write!(out, " {} {}", token(name, "00"), ll(*lat, *lon));
        }
        out.push_str(EOL);
    }
    for h in &ap.helipads {
        let _ = write!(
            out,
            "{} {} {} {:.2} {:.2} {:.2} {} {} {} {:.2} {}{EOL}",
            row::HELIPAD,
            token(&h.name, "H1"),
            ll(h.lat, h.lon),
            hdg(h.heading),
            h.length_m,
            h.width_m,
            h.surface,
            h.markings,
            h.shoulder,
            h.smoothness,
            h.edge_lights
        );
    }

    if ap.boundary.len() >= 3 {
        let _ = write!(out, "{} Airport Boundary{EOL}", row::BOUNDARY);
        write_ring(out, &ap.boundary, true);
    }
    for p in &ap.pavements {
        let rings: Vec<&Vec<Node>> = p.rings.iter().filter(|r| r.len() >= 3).collect();
        if rings.is_empty() {
            continue;
        }
        let _ = write!(
            out,
            "{} {} {:.2} {:.2} {}{EOL}",
            row::PAVEMENT,
            p.surface,
            p.smoothness,
            hdg(p.heading),
            text(&p.name)
        );
        for ring in rings {
            write_ring(out, ring, true);
        }
    }

    for l in &ap.lines {
        let min = if l.closed { 3 } else { 2 };
        if l.nodes.len() < min {
            continue;
        }
        let _ = write!(out, "{} {}{EOL}", row::LINEAR_FEATURE, text(&l.name));
        write_ring(out, &l.nodes, l.closed);
    }

    if let Some(t) = &ap.tower {
        let _ = write!(
            out,
            "{} {} {:.0} {} {}{EOL}",
            row::TOWER,
            ll(t.lat, t.lon),
            t.height_ft.max(0.0),
            u8::from(t.draw),
            text(&t.name)
        );
    }
    for b in &ap.beacons {
        let _ = write!(
            out,
            "{} {} {} {}{EOL}",
            row::BEACON,
            ll(b.lat, b.lon),
            b.kind,
            text(&b.name)
        );
    }
    for w in &ap.windsocks {
        let _ = write!(
            out,
            "{} {} {} {}{EOL}",
            row::WINDSOCK,
            ll(w.lat, w.lon),
            u8::from(w.lit),
            text(&w.name)
        );
    }
    for s in &ap.signs {
        let _ = write!(
            out,
            "{} {} {:.2} 0 {} {}{EOL}",
            row::SIGN,
            ll(s.lat, s.lon),
            hdg(s.heading),
            s.size.clamp(1, 5),
            token(&s.text, "{@Y}X")
        );
    }
    for l in &ap.light_objects {
        let _ = write!(
            out,
            "{} {} {} {:.2} {:.2} {} {}{EOL}",
            row::LIGHT_OBJECT,
            ll(l.lat, l.lon),
            l.kind,
            hdg(l.heading),
            l.angle,
            token(&l.runway, "00"),
            text(&l.description)
        );
    }
    for f in &ap.frequencies {
        let _ = write!(out, "{} {:06} {}{EOL}", f.code, f.khz, text(&f.name));
    }

    if let Some(net) = &ap.network {
        if !net.nodes.is_empty() {
            let _ = write!(out, "{} {}{EOL}", row::TAXI_NETWORK, text(&net.name));
            for n in &net.nodes {
                let _ = write!(
                    out,
                    "{} {} {} {} {}{EOL}",
                    row::TAXI_NODE,
                    ll(n.lat, n.lon),
                    n.usage,
                    n.id,
                    text(&n.name)
                );
            }
            for e in &net.edges {
                let _ = write!(
                    out,
                    "{} {} {} {} {} {}{EOL}",
                    row::TAXI_EDGE,
                    e.from,
                    e.to,
                    if e.oneway { "oneway" } else { "twoway" },
                    e.kind,
                    text(&e.name)
                );
                for &(lat, lon) in &e.shape {
                    let _ = write!(out, "{} {}{EOL}", row::TAXI_SHAPE, ll(lat, lon));
                }
                for z in &e.active_zones {
                    if !z.runways.is_empty() {
                        let _ = write!(
                            out,
                            "{} {} {}{EOL}",
                            row::TAXI_ACTIVE_ZONE,
                            z.phase,
                            z.runways.join(",")
                        );
                    }
                }
            }
            for e in &net.truck_edges {
                let _ = write!(
                    out,
                    "{} {} {} {} {}{EOL}",
                    row::TAXI_TRUCK_EDGE,
                    e.from,
                    e.to,
                    if e.oneway { "oneway" } else { "twoway" },
                    text(&e.name)
                );
                for &(lat, lon) in &e.shape {
                    let _ = write!(out, "{} {}{EOL}", row::TAXI_SHAPE, ll(lat, lon));
                }
            }
        }
    }

    for r in &ap.ramp_starts {
        let aircraft = if r.aircraft.is_empty() {
            "all".to_string()
        } else {
            r.aircraft.join("|")
        };
        let _ = write!(
            out,
            "{} {} {:.2} {} {} {}{EOL}",
            row::RAMP_START,
            ll(r.lat, r.lon),
            hdg(r.heading),
            r.location,
            aircraft,
            text(&r.name)
        );
        let airlines: Vec<String> = r
            .airlines
            .iter()
            .map(|a| token(a, "").to_ascii_lowercase())
            .filter(|a| a.len() == 3)
            .collect();
        let _ = write!(out, "{} {} {}", row::RAMP_START_META, r.width, r.operations);
        if !airlines.is_empty() {
            let _ = write!(out, " {}", airlines.join(" "));
        }
        out.push_str(EOL);
    }
    for j in &ap.jetways {
        let _ = write!(
            out,
            "{} {} {:.1} {} {} {:.1} {:.2} {:.1}{EOL}",
            row::JETWAY,
            ll(j.lat, j.lon),
            hdg(j.install_heading),
            j.style,
            j.size,
            hdg(j.parked_tunnel_heading),
            j.parked_tunnel_length,
            hdg(j.parked_cab_heading)
        );
    }
    for t in &ap.truck_parking {
        let _ = write!(
            out,
            "{} {} {:.2} {} {} {}{EOL}",
            row::TRUCK_PARKING,
            ll(t.lat, t.lon),
            hdg(t.heading),
            t.kind,
            t.cars,
            text(&t.name)
        );
    }
}

/// The whole file, including the platform line, version header and `99` footer.
pub fn write(apt: &Apt) -> String {
    let mut out = String::with_capacity(1 << 20);
    let _ = write!(
        out,
        "I{EOL}1200 Generated by msfs2xp {} - converted from Microsoft Flight Simulator{EOL}",
        env!("CARGO_PKG_VERSION")
    );
    for ap in &apt.airports {
        write_airport(&mut out, ap);
    }
    let _ = write!(out, "{EOL}{}{EOL}", row::FILE_END);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Apt {
        let end = |name: &str, lat: f64| RunwayEnd {
            name: name.into(),
            lat,
            lon: 55.36,
            markings: 3,
            approach_lights: 1,
            touchdown_lights: true,
            reil: 0,
            ..Default::default()
        };
        let sq = |lat: f64| {
            vec![
                Node::at(lat, 55.0),
                Node::at(lat + 0.001, 55.0),
                Node::at(lat + 0.001, 55.001),
                Node::at(lat, 55.001),
            ]
        };
        Apt {
            airports: vec![AptAirport {
                kind: row::LAND_AIRPORT,
                elevation_ft: 62,
                icao: "OMDB".into(),
                name: "Dubai  Intl".into(),
                metadata: vec![("icao_code".into(), "OMDB".into())],
                runways: vec![Runway {
                    width_m: 60.0,
                    surface: 1,
                    smoothness: 0.25,
                    centre_lights: true,
                    edge_lights: 3,
                    ends: [end("12L", 25.26), end("30R", 25.24)],
                    ..Default::default()
                }],
                pavements: vec![Pavement {
                    surface: 1,
                    smoothness: 0.25,
                    name: "Apron".into(),
                    rings: vec![sq(25.0), sq(25.0002)],
                    ..Default::default()
                }],
                lines: vec![LinearFeature {
                    name: "centre".into(),
                    nodes: vec![
                        Node {
                            line: 1,
                            light: 101,
                            ..Node::at(25.0, 55.0)
                        },
                        Node::at(25.001, 55.0),
                    ],
                    closed: false,
                }],
                frequencies: vec![Frequency {
                    code: row::FREQ_TOWER,
                    khz: 118_750,
                    name: "DUBAI TOWER".into(),
                }],
                network: Some(Network {
                    name: "OMDB".into(),
                    nodes: vec![
                        NetNode {
                            lat: 25.0,
                            lon: 55.0,
                            usage: "junc".into(),
                            id: 0,
                            name: "n0".into(),
                        },
                        NetNode {
                            lat: 25.001,
                            lon: 55.0,
                            usage: "both".into(),
                            id: 1,
                            name: "n1".into(),
                        },
                    ],
                    edges: vec![NetEdge {
                        from: 0,
                        to: 1,
                        kind: "taxiway_E".into(),
                        name: "M".into(),
                        active_zones: vec![ActiveZone {
                            phase: "departure".into(),
                            runways: vec!["12L".into(), "30R".into()],
                        }],
                        ..Default::default()
                    }],
                    truck_edges: vec![],
                }),
                ramp_starts: vec![RampStart {
                    lat: 25.0,
                    lon: 55.0,
                    heading: 360.0,
                    location: "gate".into(),
                    aircraft: vec!["heavy".into(), "jets".into()],
                    name: "A12".into(),
                    width: 'E',
                    operations: "airline".into(),
                    airlines: vec!["UAE".into(), "toolong".into()],
                }],
                jetways: vec![Jetway {
                    lat: 25.0,
                    lon: 55.0,
                    install_heading: 90.0,
                    size: 1,
                    parked_tunnel_length: 12.0,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    #[test]
    fn writes_header_airport_and_footer() {
        let s = write(&sample());
        assert!(s.starts_with("I\r\n1200 Generated by msfs2xp"));
        assert!(
            s.contains("\r\n1 62 0 0 OMDB Dubai Intl\r\n"),
            "names collapse whitespace:\n{s}"
        );
        assert!(s.contains("1302 icao_code OMDB\r\n"));
        assert!(s.trim_end().ends_with("99"));
    }

    #[test]
    fn runway_row_has_every_column() {
        let s = write(&sample());
        let row = s.lines().find(|l| l.starts_with("100 ")).unwrap();
        // 8 shared columns plus 9 per end.
        assert_eq!(row.split_whitespace().count(), 8 + 9 * 2, "{row}");
        assert!(row.contains(" 12L 25.26000000 55.36000000 0.00 0.00 3 1 1 0"));
    }

    #[test]
    fn pavement_rings_close_and_holes_follow() {
        let s = write(&sample());
        let rows: Vec<&str> = s.lines().collect();
        let start = rows.iter().position(|l| l.starts_with("110 ")).unwrap();
        let codes: Vec<&str> = rows[start + 1..start + 9]
            .iter()
            .map(|l| l.split(' ').next().unwrap())
            .collect();
        assert_eq!(codes, vec!["111", "111", "111", "113", "111", "111", "111", "113"]);
    }

    #[test]
    fn open_lines_end_without_attributes() {
        let s = write(&sample());
        let rows: Vec<&str> = s.lines().collect();
        let start = rows.iter().position(|l| l.starts_with("120 ")).unwrap();
        assert_eq!(rows[start + 1], "111 25.00000000 55.00000000 1 101");
        assert_eq!(rows[start + 2], "115 25.00100000 55.00000000");
    }

    #[test]
    fn network_and_ramps_render() {
        let s = write(&sample());
        assert!(s.contains("1202 0 1 twoway taxiway_E M\r\n1204 departure 12L,30R\r\n"));
        assert!(s.contains("1054 118750 DUBAI TOWER"));
        assert!(
            s.contains("1300 25.00000000 55.00000000 0.00 gate heavy|jets A12"),
            "360 wraps to 0"
        );
        assert!(
            s.contains("1301 E airline uae\r\n"),
            "invalid airline codes are dropped"
        );
        assert!(s.contains("1500 25.00000000 55.00000000 90.0 0 1 0.0 12.00 0.0"));
    }

    #[test]
    fn degenerate_features_are_skipped() {
        let mut apt = sample();
        apt.airports[0].pavements[0].rings = vec![vec![Node::at(1.0, 1.0), Node::at(1.0, 2.0)]];
        apt.airports[0].lines[0].nodes.truncate(1);
        let s = write(&apt);
        assert!(!s.contains("\r\n110 "));
        assert!(!s.contains("\r\n120 "));
    }
}
