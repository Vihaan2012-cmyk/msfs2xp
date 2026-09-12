//! Human-readable dump of a BGL's record tree.
//!
//! This is the tool used to work out undocumented layouts: it shows every
//! record, its size, and for records we do not interpret, a hex dump annotated
//! with any 8-byte window that decodes to a coordinate near the airport.

use std::fmt::Write as _;

use crate::bgl::codec::{icao_from_u32, lat_from_u32, lon_from_u32};
use crate::bgl::file::{BglFile, FileClass, SubRecords};
use crate::bgl::records::geoscan::{read_pair, Near};
use crate::bgl::records::ids::{self, *};
use crate::bgl::records::parse_airport;

/// Options for the dump.
#[derive(Debug, Clone, Copy, Default)]
pub struct InspectOptions {
    pub hex: bool,
}

/// Render the record tree of `data`.
pub fn inspect(data: &[u8], icao: Option<&str>, opts: InspectOptions) -> String {
    let mut out = String::new();
    match crate::bgl::classify(data) {
        FileClass::Bgl => {}
        FileClass::Encrypted => {
            return "file is not a readable BGL: content looks encrypted (marketplace DRM)\n".into();
        }
        FileClass::Truncated => return "file is a truncated BGL\n".into(),
        FileClass::NotBgl => return "file is not a BGL\n".into(),
    }

    let file = match BglFile::parse(data) {
        Ok(f) => f,
        Err(e) => return format!("cannot parse BGL: {e}\n"),
    };

    let _ = writeln!(out, "sections: {}", file.sections.len());
    for s in &file.sections {
        let _ = writeln!(
            out,
            "  section 0x{:04X} ({}) subsections {}",
            s.kind,
            section_name(s.kind),
            s.subsections.len()
        );
    }

    for rec in file.airport_records() {
        // Decode just the identifier and position so we can filter and so the
        // coordinate annotator has a reference point.
        let ident_at = rec.offset + 6 + 34;
        let ident = if ident_at + 4 <= data.len() {
            icao_from_u32(
                u32::from_le_bytes([
                    data[ident_at],
                    data[ident_at + 1],
                    data[ident_at + 2],
                    data[ident_at + 3],
                ]),
                true,
            )
        } else {
            String::new()
        };
        let lon = lon_from_u32(u32::from_le_bytes([
            rec.data[12],
            rec.data[13],
            rec.data[14],
            rec.data[15],
        ]));
        let lat = lat_from_u32(u32::from_le_bytes([
            rec.data[16],
            rec.data[17],
            rec.data[18],
            rec.data[19],
        ]));
        let near = Near::new(lat, lon);

        let parsed = parse_airport(&rec, None).ok();
        let variant = parsed.as_ref().map(|a| a.variant).unwrap_or_default();
        // MSFS 2024 moved the ident out of the common head, so prefer the parser's.
        let ident = parsed
            .as_ref()
            .map(|a| a.ident.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or(ident);
        if let Some(want) = icao {
            if !ident.eq_ignore_ascii_case(want) {
                continue;
            }
        }
        let _ = writeln!(
            out,
            "\nAIRPORT {ident} at {lat:.6},{lon:.6}  offset 0x{:X}  size {}  layout {}",
            rec.offset,
            rec.size(),
            variant.label()
        );
        if let Some(ap) = &parsed {
            let _ = writeln!(
                out,
                "  name {:?}  runways {} parkings {} taxi points {} paths {} aprons {} lines {} signs {}",
                ap.name,
                ap.runways.len(),
                ap.parkings.len(),
                ap.taxi_points.len(),
                ap.taxi_paths.len(),
                ap.aprons.len(),
                ap.painted_lines.len(),
                ap.signs.len()
            );
            for w in &ap.warnings {
                let _ = writeln!(out, "  ! {w}");
            }
        }

        // Walk sub-records from the detected head so offsets line up with the parser.
        let head = detect_head(rec.data);
        for sub in SubRecords::new(rec.data, head) {
            let known = ids::is_airport_subrecord(sub.id);
            let _ = writeln!(
                out,
                "  0x{:04X} {:<22} size {:<6} at 0x{:X}{}",
                sub.id,
                airport_subrecord_name(sub.id),
                sub.size(),
                rec.offset + sub.offset,
                if known { "" } else { "   <-- UNKNOWN" }
            );
            if sub.id == AP_RUNWAY || sub.id == AP_RUNWAY_MSFS || sub.id == AP_RUNWAY_P3D_V4 {
                for rsub in SubRecords::new(sub.data, runway_head(sub.data)) {
                    let _ = writeln!(
                        out,
                        "      0x{:04X} {:<28} size {}",
                        rsub.id,
                        runway_subrecord_name(rsub.id),
                        rsub.size()
                    );
                }
            }
            if opts.hex && (!known || is_undocumented(sub.id)) {
                out.push_str(&hexdump(sub.data, &near, 6));
            }
        }
    }
    out
}

fn is_undocumented(id: u16) -> bool {
    matches!(
        id,
        AP_MSFS_PAINTED_LINE
            | AP_MSFS_TAXIWAY_SIGN
            | AP_APRON_EDGE_LIGHTS
            | AP_MSFS_PAINTED_HATCHED_AREA
            | AP_MSFS_JETWAY
            | AP_MSFS_LIGHT_SUPPORT
    )
}

fn detect_head(data: &[u8]) -> usize {
    let candidates: Vec<usize> = {
        let mut v = vec![6 + 62, 6 + 50, 6 + 58, 6 + 46];
        v.extend((4..=32).step_by(4).map(|e| 6 + 62 + e));
        v
    };
    crate::bgl::records::layout::probe_subrecord_start(data, &candidates, ids::is_airport_subrecord)
        .map(|p| p.start)
        .unwrap_or(6 + 50)
}

fn runway_head(data: &[u8]) -> usize {
    let mut v = vec![6 + 46, 6 + 62, 6 + 90];
    v.extend((4..=64).step_by(4).map(|e| 6 + 90 + e));
    crate::bgl::records::layout::probe_subrecord_start(data, &v, ids::is_runway_subrecord)
        .map(|p| p.start)
        .unwrap_or(data.len())
}

/// Hex dump with coordinate annotations, starting at `from`.
fn hexdump(data: &[u8], near: &Near, from: usize) -> String {
    let mut out = String::new();
    let mut i = from;
    while i < data.len() {
        let end = (i + 16).min(data.len());
        let _ = write!(out, "      {:04X}  ", i);
        for slot in 0..16 {
            match data.get(i + slot).filter(|_| i + slot < end) {
                Some(b) => {
                    let _ = write!(out, "{b:02X} ");
                }
                None => out.push_str("   "),
            }
        }
        out.push(' ');
        for &b in &data[i..end] {
            out.push(if (0x20..0x7F).contains(&b) { b as char } else { '.' });
        }
        // Annotate any coordinate pair or plausible float starting in this row:
        // this is what makes an unknown record's structure readable at a glance.
        for j in i..end {
            if let Some((lat, lon)) = read_pair(data, j) {
                if near.accepts(lat, lon) {
                    let _ = write!(out, "   @{j:04X}=({lat:.6},{lon:.6})");
                }
            }
            if let Some(chunk) = data.get(j..j + 4) {
                let f = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                if f.is_normal() && (0.1..=10000.0).contains(&f.abs()) {
                    let _ = write!(out, "   f{j:04X}={f:.3}");
                }
            }
        }
        out.push('\n');
        i = end;
    }
    out
}

fn section_name(kind: u32) -> &'static str {
    match kind {
        0x01 => "COPYRIGHT",
        0x02 => "GUID",
        0x03 => "AIRPORT",
        0x13 => "ILS_VOR",
        0x17 => "NDB",
        0x18 => "MARKER",
        0x20 => "BOUNDARY",
        0x22 => "WAYPOINT",
        0x25 => "SCENERY_OBJECT",
        0x27 => "NAME_LIST",
        0x2B => "MODEL_DATA",
        0x2C => "AIRPORT_SUMMARY",
        0x2E => "EXCLUSION",
        0x2F => "TIMEZONE",
        0xDA => "MSFS_DELETE_NAV",
        0xDB => "MSFS_DELETE_AIRPORT_NAV",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgl::codec::icao_to_u32;
    use crate::bgl::file::SECTION_AIRPORT;
    use crate::bgl::testutil::{record, BglBuilder, Bytes};

    #[test]
    fn prints_tree_for_builder_file() {
        let mut body = Bytes::new()
            .zeros(6)
            .pos(25.25, 55.36, 19.0)
            .pos(25.25, 55.36, 60.0)
            .f32(0.0)
            .u32(icao_to_u32("OMDB", true))
            .u32(0)
            .u32(0)
            .zeros(4)
            .done();
        body.extend_from_slice(&record(AP_NAME, b"Dubai"));
        let data = BglBuilder::new()
            .section(SECTION_AIRPORT, vec![record(REC_AIRPORT, &body)])
            .build();
        let text = inspect(&data, None, InspectOptions::default());
        assert!(text.contains("AIRPORT OMDB"), "{text}");
        assert!(text.contains("NAME"), "{text}");
    }

    #[test]
    fn reports_non_bgl() {
        assert!(inspect(b"plain text, not a scenery file", None, InspectOptions::default()).contains("not a BGL"));
    }
}
