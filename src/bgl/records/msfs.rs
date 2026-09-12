//! MSFS-only records whose layout Asobo has never published: painted lines and
//! taxiway signs.
//!
//! Both are located structurally (see `geoscan`) rather than by a fixed offset
//! table, so they survive the field insertions between MSFS 2020 and 2024.
//! Anything that cannot be identified confidently is left alone rather than
//! guessed at, and the caller is told about it.

use crate::bgl::file::RecordSlice;
use crate::bgl::reader::BglError;

use crate::bgl::guid::Guid;

use super::geoscan::{find_vertex_run, read_pair, read_vertices, Near};
use super::raw::{RawPaintedLine, RawSign};

/// Painted line (`0x00CF`): a type code and a polyline.
pub fn parse_painted_line(
    rec: &RecordSlice,
    near: &Near,
    warnings: &mut Vec<String>,
) -> Result<RawPaintedLine, BglError> {
    let data = rec.data;
    // Every MSFS 2020 and 2024 sample so far: line type u8 at 6, "true angle"
    // u8 at 7, vertex count u16 at 8, material GUID at 12..28, vertices from 28.
    if data.len() >= 28 {
        let count = u16::from_le_bytes([data[8], data[9]]) as usize;
        if count >= 2 && 28 + count * 8 == data.len() {
            let vertices = read_vertices(data, 28, count);
            if vertices.iter().all(|&(lat, lon)| near.accepts(lat, lon)) {
                return Ok(RawPaintedLine {
                    kind: data[6] as u16,
                    true_angle: data[7],
                    material: Guid::from_slice(&data[12..28]).filter(|g| !g.is_nil()),
                    vertices,
                });
            }
        }
    }
    // Otherwise locate the vertices structurally; the style is then unknown.
    let (start, count) = match find_vertex_run(data, near, 2) {
        Some(v) => v,
        None => {
            warnings.push("painted line: no vertex run found".into());
            return Ok(RawPaintedLine::default());
        }
    };
    Ok(RawPaintedLine {
        kind: 0,
        true_angle: 0,
        material: None,
        vertices: read_vertices(data, start, count),
    })
}

/// Taxiway sign (`0x00D9`): a position, a heading, a size and a label string.
pub fn parse_sign(rec: &RecordSlice, near: &Near, warnings: &mut Vec<String>) -> Result<RawSign, BglError> {
    let data = rec.data;
    let mut found = None;
    let mut at = 6;
    while at + 8 <= data.len() {
        if let Some((lat, lon)) = read_pair(data, at) {
            if near.accepts(lat, lon) {
                found = Some((at, lat, lon));
                break;
            }
        }
        at += 1; // sign records are small, so a byte-wise scan is cheap
    }
    let (pos_at, lat, lon) = match found {
        Some(v) => v,
        None => {
            warnings.push("taxiway sign: no position found".into());
            return Ok(RawSign::default());
        }
    };

    // Heading follows the position (after the altitude). `is_normal` matters:
    // a millimetre altitude reinterpreted as a float is a subnormal value that
    // would otherwise look like a plausible heading.
    let mut heading = 0.0f32;
    for off in (pos_at + 8..(pos_at + 28).min(data.len().saturating_sub(4))).step_by(4) {
        let v = f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
        if v.is_normal() && (0.01..=360.0).contains(&v) {
            heading = v;
            break;
        }
    }

    let (label, size) = find_label(data, pos_at).unwrap_or_default();
    if label.is_empty() {
        warnings.push("taxiway sign: no label found".into());
    }
    Ok(RawSign {
        lat,
        lon,
        heading,
        size,
        justification: 0,
        label,
    })
}

/// Characters that are legal inside an MSFS sign label.
fn is_label_byte(b: u8) -> bool {
    matches!(b,
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' |
        b'[' | b']' | b'<' | b'>' | b'^' | b'_' | b'-' | b'|' | b'/' | b'\\' | b'\'' | b'`' | b' ')
}

/// The longest run of label characters that opens with a sign-type letter.
fn find_label(data: &[u8], after: usize) -> Option<(String, u8)> {
    let mut best: Option<(usize, usize)> = None;
    let mut i = 6;
    while i < data.len() {
        if is_label_byte(data[i]) {
            let start = i;
            while i < data.len() && is_label_byte(data[i]) {
                i += 1;
            }
            let len = i - start;
            let opens_ok = matches!(data[start], b'l' | b'd' | b'm' | b'i' | b'r' | b'u');
            if len >= 2 && opens_ok && best.map(|(_, bl)| len > bl).unwrap_or(true) {
                best = Some((start, len));
            }
        } else {
            i += 1;
        }
    }
    let (start, len) = best?;
    let label = String::from_utf8_lossy(&data[start..start + len]).to_string();
    // Sign size: a byte in 1..=5 just before the label or just after the position.
    let mut size = 3u8;
    for probe in [start.wrapping_sub(1), start.wrapping_sub(2), after + 12, after + 13] {
        if probe < data.len() && (1..=5).contains(&data[probe]) {
            size = data[probe];
            break;
        }
    }
    Some((label, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgl::records::ids::*;
    use crate::bgl::testutil::{record, Bytes};

    const LAT: f64 = 25.2528;
    const LON: f64 = 55.3644;

    #[test]
    fn reads_the_structured_painted_line_layout() {
        let guid = [
            0xD6u8, 0xE1, 0x30, 0x82, 0x0C, 0x68, 0xB9, 0x4E, 0xA9, 0xFD, 0x01, 0xF9, 0xA3, 0xEF, 0xE3, 0x18,
        ];
        let body = Bytes::new()
            .u8(7)
            .u8(3)
            .u16(3)
            .u16(0)
            .raw(&guid)
            .pos2(LAT, LON)
            .pos2(LAT + 0.001, LON)
            .pos2(LAT + 0.002, LON + 0.001)
            .done();
        let bytes = record(AP_MSFS_PAINTED_LINE, &body);
        let rec = RecordSlice {
            id: AP_MSFS_PAINTED_LINE,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let line = parse_painted_line(&rec, &Near::new(LAT, LON), &mut w).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((line.kind, line.true_angle), (7, 3));
        assert_eq!(line.material, Some(Guid(guid)));
        assert_eq!(line.vertices.len(), 3);
        assert!((line.vertices[2].1 - (LON + 0.001)).abs() < 1e-6);
    }

    #[test]
    fn falls_back_to_scanning_when_the_layout_differs() {
        let body = Bytes::new()
            .u16(6)
            .u16(4)
            .u16(3)
            .pos2(LAT, LON)
            .pos2(LAT + 0.001, LON)
            .pos2(LAT + 0.002, LON + 0.001)
            .done();
        let bytes = record(AP_MSFS_PAINTED_LINE, &body);
        let rec = RecordSlice {
            id: AP_MSFS_PAINTED_LINE,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let line = parse_painted_line(&rec, &Near::new(LAT, LON), &mut w).unwrap();
        assert_eq!(line.vertices.len(), 3);
        assert!(line.material.is_none());
    }

    #[test]
    fn painted_line_far_away_is_rejected() {
        let body = Bytes::new().u16(0).pos2(0.0, 0.0).pos2(0.1, 0.1).done();
        let bytes = record(AP_MSFS_PAINTED_LINE, &body);
        let rec = RecordSlice {
            id: AP_MSFS_PAINTED_LINE,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let line = parse_painted_line(&rec, &Near::new(LAT, LON), &mut w).unwrap();
        assert!(line.vertices.is_empty());
        assert!(!w.is_empty());
    }

    #[test]
    fn finds_sign_position_heading_and_label() {
        let mut label = b"l[G]d[F".to_vec();
        label.push(b'\\');
        label.extend_from_slice(b"]m[12R-30L]");
        let body = Bytes::new()
            .u8(2) // size
            .u8(0)
            .pos2(LAT, LON)
            .i32(19_000) // altitude in mm
            .f32(287.5) // heading
            .u16(label.len() as u16)
            .raw(&label)
            .done();
        let bytes = record(AP_MSFS_TAXIWAY_SIGN, &body);
        let rec = RecordSlice {
            id: AP_MSFS_TAXIWAY_SIGN,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let sign = parse_sign(&rec, &Near::new(LAT, LON), &mut w).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert!((sign.lat - LAT).abs() < 1e-6);
        assert!((sign.lon - LON).abs() < 1e-6);
        assert!((sign.heading - 287.5).abs() < 0.01);
        assert_eq!(sign.label, String::from_utf8_lossy(&label));
    }

    #[test]
    fn sign_without_label_warns() {
        let body = Bytes::new().pos2(LAT, LON).i32(0).f32(90.0).done();
        let bytes = record(AP_MSFS_TAXIWAY_SIGN, &body);
        let rec = RecordSlice {
            id: AP_MSFS_TAXIWAY_SIGN,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let sign = parse_sign(&rec, &Near::new(LAT, LON), &mut w).unwrap();
        assert!(sign.label.is_empty());
        assert!(!w.is_empty());
    }
}
