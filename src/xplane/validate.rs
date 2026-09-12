//! A strict structural check of apt.dat text.
//!
//! X-Plane reports a malformed airport file by silently dropping the airport, and
//! WorldEditor by refusing to open the pack, so the converter checks its own
//! output before anyone sees it. This is not a full reimplementation of
//! X-Plane's loader; it checks the things a generator can realistically get
//! wrong: column counts, value ranges, polygon and line termination, and taxi
//! network references.

use std::collections::HashSet;

/// What a successful validation found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    pub airports: usize,
    pub rows: usize,
    pub runways: usize,
    pub pavements: usize,
    pub lines: usize,
    pub network_nodes: usize,
    pub network_edges: usize,
    pub ramp_starts: usize,
    pub jetways: usize,
}

#[derive(PartialEq)]
enum Chain {
    None,
    Pavement { nodes_in_ring: usize, rings: usize },
    Line { nodes: usize },
}

fn num(tok: Option<&str>) -> Option<f64> {
    tok.and_then(|t| t.parse::<f64>().ok()).filter(|v| v.is_finite())
}

fn lat_ok(v: Option<f64>) -> bool {
    matches!(v, Some(x) if (-90.0..=90.0).contains(&x))
}

fn lon_ok(v: Option<f64>) -> bool {
    matches!(v, Some(x) if (-180.0..=180.0).contains(&x))
}

/// Validate an apt.dat document. Returns every problem found, not just the first.
pub fn validate(text: &str) -> Result<ValidationReport, Vec<String>> {
    let mut errors: Vec<String> = Vec::new();
    let mut report = ValidationReport::default();
    let mut lines = text.lines().enumerate();

    match lines.next() {
        Some((_, l)) if matches!(l.trim(), "I" | "A") => {}
        _ => errors.push("line 1: expected 'I' or 'A'".into()),
    }
    match lines.next() {
        Some((_, l)) if l.split_whitespace().next().map(|v| v.parse::<u32>().is_ok()) == Some(true) => {
            let v: u32 = l.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0);
            if v < 1000 {
                errors.push(format!("line 2: version {v} is older than this validator understands"));
            }
        }
        _ => errors.push("line 2: expected a version number".into()),
    }

    let mut chain = Chain::None;
    let mut in_airport = false;
    let mut node_ids: HashSet<u64> = HashSet::new();
    let mut edges: Vec<(usize, u64, u64)> = Vec::new();
    let mut saw_end = false;
    let mut last_was_edge = false;

    let finish_chain = |chain: &mut Chain, errors: &mut Vec<String>, at: usize| {
        match chain {
            Chain::Pavement { nodes_in_ring, rings } if *nodes_in_ring > 0 || *rings == 0 => {
                errors.push(format!("line {at}: pavement ring not closed with 113/114"));
            }
            Chain::Line { nodes } if *nodes > 0 => {
                errors.push(format!("line {at}: linear feature not terminated with 113-116"));
            }
            _ => {}
        }
        *chain = Chain::None;
    };

    let check_network = |node_ids: &HashSet<u64>, edges: &[(usize, u64, u64)], errors: &mut Vec<String>| {
        for &(line, a, b) in edges {
            for n in [a, b] {
                if !node_ids.contains(&n) {
                    errors.push(format!("line {line}: taxi edge references missing node {n}"));
                }
            }
        }
    };

    for (i, raw) in lines {
        let lineno = i + 1;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let mut cols = line.split_whitespace();
        let Some(code) = cols.next().and_then(|c| c.parse::<u32>().ok()) else {
            errors.push(format!("line {lineno}: row does not start with a numeric code"));
            continue;
        };
        report.rows += 1;
        let rest: Vec<&str> = cols.collect();
        let is_node = (111..=116).contains(&code);

        if !is_node && chain != Chain::None {
            finish_chain(&mut chain, &mut errors, lineno);
        }
        if code != 1203 && code != 1204 {
            last_was_edge = false;
        }

        match code {
            1 | 16 | 17 => {
                check_network(&node_ids, &edges, &mut errors);
                node_ids.clear();
                edges.clear();
                in_airport = true;
                report.airports += 1;
                if rest.len() < 4 || num(rest.first().copied()).is_none() {
                    errors.push(format!(
                        "line {lineno}: airport header needs elevation, two flags and an ICAO"
                    ));
                }
            }
            99 => {
                saw_end = true;
                check_network(&node_ids, &edges, &mut errors);
                break;
            }
            _ if !in_airport => errors.push(format!("line {lineno}: row {code} before any airport header")),
            100 => {
                report.runways += 1;
                if rest.len() != 7 + 9 * 2 {
                    errors.push(format!(
                        "line {lineno}: runway row has {} columns, expected 25",
                        rest.len()
                    ));
                } else {
                    for end in 0..2 {
                        let base = 7 + end * 9;
                        if !lat_ok(num(rest.get(base + 1).copied())) || !lon_ok(num(rest.get(base + 2).copied())) {
                            errors.push(format!("line {lineno}: runway end {} position out of range", end + 1));
                        }
                    }
                    let surface = num(rest.get(1).copied()).unwrap_or(-1.0) as i64;
                    if !matches!(surface, 1..=5 | 12..=15 | 20..=38 | 50..=57) {
                        errors.push(format!("line {lineno}: unknown runway surface {surface}"));
                    }
                }
            }
            101 => {
                if rest.len() != 8 {
                    errors.push(format!("line {lineno}: water runway needs 8 columns"));
                }
            }
            102 => {
                if rest.len() != 11 || !lat_ok(num(rest.get(1).copied())) || !lon_ok(num(rest.get(2).copied())) {
                    errors.push(format!("line {lineno}: helipad row malformed"));
                }
            }
            110 => {
                report.pavements += 1;
                if rest.len() < 3 {
                    errors.push(format!(
                        "line {lineno}: pavement header needs surface, smoothness, heading"
                    ));
                }
                chain = Chain::Pavement {
                    nodes_in_ring: 0,
                    rings: 0,
                };
            }
            120 => {
                report.lines += 1;
                chain = Chain::Line { nodes: 0 };
            }
            130 => chain = Chain::Line { nodes: 0 },
            111..=116 => {
                let need = if matches!(code, 112 | 114 | 116) { 4 } else { 2 };
                if rest.len() < need || !lat_ok(num(rest.first().copied())) || !lon_ok(num(rest.get(1).copied())) {
                    errors.push(format!("line {lineno}: node row malformed"));
                }
                match &mut chain {
                    Chain::None => errors.push(format!("line {lineno}: node outside a pavement or line")),
                    Chain::Pavement { nodes_in_ring, rings } => {
                        *nodes_in_ring += 1;
                        if matches!(code, 113 | 114) {
                            if *nodes_in_ring < 3 {
                                errors.push(format!("line {lineno}: pavement ring with fewer than 3 nodes"));
                            }
                            *nodes_in_ring = 0;
                            *rings += 1;
                        } else if matches!(code, 115 | 116) {
                            errors.push(format!("line {lineno}: pavement rings must close, not end"));
                        }
                    }
                    Chain::Line { nodes } => {
                        *nodes += 1;
                        if (113..=116).contains(&code) {
                            if *nodes < 2 {
                                errors.push(format!("line {lineno}: line with a single node"));
                            }
                            *nodes = 0;
                        }
                    }
                }
            }
            14 | 18 | 19 => {
                if !lat_ok(num(rest.first().copied())) || !lon_ok(num(rest.get(1).copied())) {
                    errors.push(format!("line {lineno}: row {code} position out of range"));
                }
            }
            20 => {
                if rest.len() < 6 {
                    errors.push(format!("line {lineno}: sign row needs 6 columns"));
                }
            }
            21 => {
                let kind = num(rest.get(2).copied()).unwrap_or(0.0) as i64;
                if rest.len() < 6 || !(1..=8).contains(&kind) {
                    errors.push(format!("line {lineno}: light object row malformed"));
                }
            }
            50..=56 | 1050..=1056 => {
                if rest.is_empty() || num(rest.first().copied()).is_none() {
                    errors.push(format!("line {lineno}: frequency row needs a frequency"));
                }
            }
            1200 => {}
            1201 => {
                let id = num(rest.get(3).copied()).map(|v| v as u64);
                if rest.len() < 4 || !matches!(rest.get(2).copied(), Some("dest" | "init" | "both" | "junc")) {
                    errors.push(format!("line {lineno}: taxi node row malformed"));
                }
                if let Some(id) = id {
                    if !node_ids.insert(id) {
                        errors.push(format!("line {lineno}: duplicate taxi node id {id}"));
                    }
                    report.network_nodes += 1;
                }
            }
            1202 | 1206 => {
                let a = num(rest.first().copied()).map(|v| v as u64);
                let b = num(rest.get(1).copied()).map(|v| v as u64);
                match (a, b, rest.get(2).copied()) {
                    (Some(a), Some(b), Some("oneway" | "twoway")) => {
                        if a == b {
                            errors.push(format!("line {lineno}: taxi edge from a node to itself"));
                        }
                        edges.push((lineno, a, b));
                        report.network_edges += 1;
                        last_was_edge = code == 1202;
                    }
                    _ => errors.push(format!("line {lineno}: taxi edge row malformed")),
                }
                if code == 1202 {
                    let kind = rest.get(3).copied().unwrap_or("");
                    let ok = kind == "runway"
                        || kind == "taxiway"
                        || kind
                            .strip_prefix("taxiway_")
                            .is_some_and(|w| matches!(w, "A" | "B" | "C" | "D" | "E" | "F"));
                    if !ok {
                        errors.push(format!(
                            "line {lineno}: taxi edge type {kind:?} is not runway or taxiway_A-F"
                        ));
                    }
                }
            }
            1203 => {}
            1204 => {
                if !last_was_edge {
                    errors.push(format!("line {lineno}: active zone not attached to a taxi edge"));
                }
                if !matches!(rest.first().copied(), Some("departure" | "arrival" | "ils")) || rest.len() < 2 {
                    errors.push(format!("line {lineno}: active zone row malformed"));
                }
            }
            1300 => {
                report.ramp_starts += 1;
                if rest.len() < 5 || !matches!(rest.get(3).copied(), Some("gate" | "hangar" | "misc" | "tie_down")) {
                    errors.push(format!("line {lineno}: ramp start row malformed"));
                }
            }
            1301 => {
                let width_ok = matches!(rest.first().copied(), Some("A" | "B" | "C" | "D" | "E" | "F"));
                let op_ok = matches!(
                    rest.get(1).copied(),
                    Some("none" | "general_aviation" | "airline" | "cargo" | "military")
                );
                if !width_ok || !op_ok {
                    errors.push(format!("line {lineno}: ramp start metadata malformed"));
                }
            }
            1302 => {
                if rest.is_empty() {
                    errors.push(format!("line {lineno}: metadata row needs a key"));
                }
            }
            1400 | 1401 | 1402 | 1501 | 1502 => {}
            1500 => {
                report.jetways += 1;
                if rest.len() != 8 || !lat_ok(num(rest.first().copied())) || !lon_ok(num(rest.get(1).copied())) {
                    errors.push(format!("line {lineno}: jetway row needs 8 columns"));
                }
            }
            1000..=1004 | 1100 | 1101 | 1110 => {}
            other => errors.push(format!("line {lineno}: unknown row code {other}")),
        }
    }

    if chain != Chain::None {
        finish_chain(&mut chain, &mut errors, 0);
    }
    if !saw_end {
        errors.push("file does not end with row 99".into());
    }
    if report.airports == 0 {
        errors.push("file contains no airport".into());
    }
    if errors.is_empty() {
        Ok(report)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "I\n1200 test\n\n1 62 0 0 OMDB Dubai\n\
        100 60.00 1 0 0.25 1 3 0 12L 25.26 55.36 0 0 3 1 1 0 30R 25.24 55.38 0 0 3 1 1 0\n\
        110 1 0.25 0 Apron\n111 25.0 55.0\n111 25.001 55.0\n113 25.001 55.001\n\
        120 line\n111 25.0 55.0 1\n115 25.001 55.0\n\
        1200 net\n1201 25.0 55.0 junc 0 a\n1201 25.001 55.0 both 1 b\n\
        1202 0 1 twoway taxiway_E M\n1204 departure 12L\n\
        1300 25 55 90 gate heavy A1\n1301 E airline uae\n\
        1500 25 55 90 0 1 0 12 0\n99\n";

    #[test]
    fn accepts_a_well_formed_file() {
        let r = validate(GOOD).unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(r.airports, 1);
        assert_eq!(r.runways, 1);
        assert_eq!(r.pavements, 1);
        assert_eq!(r.network_nodes, 2);
        assert_eq!(r.network_edges, 1);
        assert_eq!(r.jetways, 1);
    }

    #[test]
    fn catches_an_unclosed_pavement() {
        let bad = GOOD.replace("113 25.001 55.001", "111 25.001 55.001");
        assert!(validate(&bad).unwrap_err().iter().any(|e| e.contains("not closed")));
    }

    #[test]
    fn catches_a_dangling_network_reference() {
        let bad = GOOD.replace("1202 0 1 twoway", "1202 0 7 twoway");
        assert!(validate(&bad).unwrap_err().iter().any(|e| e.contains("missing node 7")));
    }

    #[test]
    fn catches_a_bad_runway_surface_and_column_count() {
        let bad = GOOD.replace("100 60.00 1 0", "100 60.00 9 0");
        assert!(validate(&bad).unwrap_err().iter().any(|e| e.contains("surface 9")));
        let bad = GOOD.replace(" 30R 25.24 55.38 0 0 3 1 1 0", "");
        assert!(validate(&bad).unwrap_err().iter().any(|e| e.contains("columns")));
    }

    #[test]
    fn catches_a_missing_footer_and_orphan_active_zone() {
        assert!(validate(&GOOD.replace("99\n", ""))
            .unwrap_err()
            .iter()
            .any(|e| e.contains("99")));
        let bad = GOOD.replace("1202 0 1 twoway taxiway_E M\n", "");
        assert!(validate(&bad).unwrap_err().iter().any(|e| e.contains("active zone")));
    }

    #[test]
    fn writer_output_validates() {
        use crate::xplane::apt::*;
        let apt = Apt {
            airports: vec![AptAirport {
                kind: 1,
                icao: "TEST".into(),
                name: "Test".into(),
                pavements: vec![Pavement {
                    surface: 2,
                    rings: vec![vec![Node::at(1.0, 1.0), Node::at(1.001, 1.0), Node::at(1.001, 1.001)]],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        validate(&crate::xplane::write::write(&apt)).unwrap_or_else(|e| panic!("{e:#?}"));
    }
}
