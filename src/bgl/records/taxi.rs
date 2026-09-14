//! Taxiway points, taxiway names, taxiway paths and parking spots.
//!
//! These are packed-array records: a `u16` element count followed by fixed (or,
//! for parking, nearly fixed) size elements. Rather than trusting one stride per
//! simulator we derive it from the record size, which absorbs the extra fields
//! MSFS 2024 appended without documentation.

use crate::bgl::codec::{lat_from_u32, lon_from_u32};
use crate::bgl::file::RecordSlice;
use crate::bgl::reader::BglError;

use super::ids::*;
use super::layout::element_stride;
use super::raw::{RawParking, RawTaxiPath, RawTaxiPoint};

const TAXI_POINT_STRIDE: usize = 12;
const TAXI_POINT_STRIDE_P3DV5: usize = 16;
const TAXI_NAME_STRIDE: usize = 8;
const TAXI_PATH_STRIDE_FSX: usize = 20;
const TAXI_PATH_STRIDE_P3DV4: usize = 36;
const TAXI_PATH_STRIDE_P3DV5: usize = 40;
const TAXI_PATH_STRIDE_MSFS: usize = 48;

fn array_header(rec: &RecordSlice) -> Result<(usize, usize), BglError> {
    let mut r = rec.body();
    let count = r.u16()? as usize;
    let available = rec.data.len().saturating_sub(8);
    Ok((count, available))
}

/// Taxi points: `u8 type`, `u8 orientation`, 2 bytes padding, `u32 lon`, `u32 lat`.
pub fn parse_taxi_points(rec: &RecordSlice, warnings: &mut Vec<String>) -> Result<Vec<RawTaxiPoint>, BglError> {
    let (count, available) = array_header(rec)?;
    let min = if rec.id == AP_TAXI_POINT_P3D_V5 {
        TAXI_POINT_STRIDE_P3DV5
    } else {
        TAXI_POINT_STRIDE
    };
    let (stride, exact) = element_stride(available, count, min);
    if !exact {
        warnings.push(format!(
            "taxi points: {count} elements do not fill {available} bytes evenly"
        ));
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let base = 8 + i * stride;
        if base + TAXI_POINT_STRIDE > rec.data.len() {
            warnings.push("taxi points: record ends early".into());
            break;
        }
        let mut r = crate::bgl::reader::Reader::at(rec.data, base);
        let kind = r.u8()?;
        let orientation = r.u8()?;
        r.skip(2)?;
        let lon = lon_from_u32(r.u32()?);
        let lat = lat_from_u32(r.u32()?);
        out.push(RawTaxiPoint {
            kind,
            orientation,
            lat,
            lon,
        });
    }
    Ok(out)
}

/// Taxi names: `u16 count` then fixed 8-byte strings (index 0 is always empty).
pub fn parse_taxi_names(rec: &RecordSlice) -> Result<Vec<String>, BglError> {
    let (count, available) = array_header(rec)?;
    let (stride, _) = element_stride(available, count, TAXI_NAME_STRIDE);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let base = 8 + i * stride;
        if base + TAXI_NAME_STRIDE > rec.data.len() {
            break;
        }
        let mut r = crate::bgl::reader::Reader::at(rec.data, base);
        out.push(r.string_fixed(TAXI_NAME_STRIDE)?);
    }
    Ok(out)
}

/// Taxi paths. The end-node index moved in MSFS: FSX packs it into the low 12
/// bits of a flag word, MSFS stores a full `u16` at the end of the element.
pub fn parse_taxi_paths(rec: &RecordSlice, warnings: &mut Vec<String>) -> Result<Vec<RawTaxiPath>, BglError> {
    let (count, available) = array_header(rec)?;
    let msfs = rec.id == AP_TAXI_PATH_MSFS;
    let min = match rec.id {
        AP_TAXI_PATH_MSFS => TAXI_PATH_STRIDE_MSFS,
        AP_TAXI_PATH_P3D_V5 => TAXI_PATH_STRIDE_P3DV5,
        AP_TAXI_PATH_P3D_V4 => TAXI_PATH_STRIDE_P3DV4,
        _ => TAXI_PATH_STRIDE_FSX,
    };
    let (stride, exact) = element_stride(available, count, min);
    if !exact {
        warnings.push(format!(
            "taxi paths: {count} elements do not fill {available} bytes evenly"
        ));
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let base = 8 + i * stride;
        if base + min.min(TAXI_PATH_STRIDE_FSX) > rec.data.len() {
            warnings.push("taxi paths: record ends early".into());
            break;
        }
        let mut r = crate::bgl::reader::Reader::at(rec.data, base);
        let start = r.u16()?;
        let flags = r.u16()?;
        let mut end = flags & 0x0FFF;
        let runway_designator = ((flags >> 12) & 0x0F) as u8;
        let f = r.u8()?;
        let kind = f & 0x0F;
        let draw_surface = f & (1 << 5) != 0;
        let draw_detail = f & (1 << 6) != 0;
        let name_index = r.u8()?;
        let f = r.u8()?;
        let centerline = f & 1 != 0;
        let centerline_lit = f & 2 != 0;
        let left_edge = (f >> 2) & 0x3;
        let left_edge_lit = f & (1 << 4) != 0;
        let right_edge = (f >> 5) & 0x3;
        let right_edge_lit = f & (1 << 7) != 0;
        let surface = r.u8()? & 0x7F;
        let width_m = r.f32()?;
        let mut material = None;
        if msfs {
            material = rec
                .data
                .get(base + 24..base + 40)
                .and_then(crate::bgl::guid::Guid::from_slice)
                .filter(|g| !g.is_nil());
            // weight limit, unknown, unknown, material GUID, unknown, then the end index
            if r.skip(4 + 4 + 4).is_ok() && r.skip(16).is_ok() && r.skip(6).is_ok() {
                if let Ok(e) = r.u16() {
                    end = e;
                }
            }
        }
        out.push(RawTaxiPath {
            start,
            end,
            runway_number: name_index,
            runway_designator,
            kind,
            draw_surface,
            draw_detail,
            name_index,
            centerline,
            centerline_lit,
            left_edge,
            left_edge_lit,
            right_edge,
            right_edge_lit,
            surface,
            width_m,
            material,
        });
    }
    Ok(out)
}

/// Parking spots. Elements are variable length because of the airline code list,
/// so the trailing block size is found by trying the known values and keeping
/// the one that consumes the record exactly.
pub fn parse_parkings(rec: &RecordSlice, warnings: &mut Vec<String>) -> Result<Vec<RawParking>, BglError> {
    let (count, _) = array_header(rec)?;
    let fs9 = rec.id == AP_TAXI_PARKING_FS9;
    let msfs = rec.id == AP_TAXI_PARKING_MSFS;
    let candidates: &[usize] = if msfs {
        // MSFS 2020 uses 20 trailing bytes; allow room for later additions.
        &[20, 24, 28, 32, 16, 36, 40, 0]
    } else if rec.id == AP_TAXI_PARKING_P3D_V5 {
        &[4, 0]
    } else {
        &[0, 4]
    };

    let mut best: Option<(Vec<RawParking>, bool)> = None;
    for &trailing in candidates {
        if let Ok((list, end)) = try_parse_parkings(rec, count, fs9, trailing) {
            let exact = end == rec.data.len();
            let better = match &best {
                None => true,
                Some((_, best_exact)) => !*best_exact && exact,
            };
            if better {
                best = Some((list, exact));
            }
            if exact {
                break;
            }
        }
    }
    match best {
        Some((list, exact)) => {
            if !exact {
                warnings.push("parkings: trailing bytes do not match a known layout".into());
            }
            Ok(list)
        }
        None => {
            warnings.push("parkings: could not parse record".into());
            Ok(Vec::new())
        }
    }
}

fn try_parse_parkings(
    rec: &RecordSlice,
    count: usize,
    fs9: bool,
    trailing: usize,
) -> Result<(Vec<RawParking>, usize), BglError> {
    let mut r = crate::bgl::reader::Reader::at(rec.data, 8);
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let flags = r.u32()?;
        let name = (flags & 0x3F) as u8;
        let push_back = ((flags >> 6) & 0x3) as u8;
        let kind = ((flags >> 8) & 0xF) as u8;
        let number = ((flags >> 12) & 0xFFF) as u16;
        let num_airlines = ((flags >> 24) & 0xFF) as usize;
        let radius_m = r.f32()?;
        let heading = r.f32()?;
        let mut tee_offsets = [0.0f32; 4];
        if !fs9 {
            for t in tee_offsets.iter_mut() {
                *t = r.f32()?;
            }
        }
        let lon = lon_from_u32(r.u32()?);
        let lat = lat_from_u32(r.u32()?);
        let mut airlines = Vec::with_capacity(num_airlines);
        for _ in 0..num_airlines {
            let code = r.string_fixed(4)?;
            if !code.is_empty() {
                airlines.push(code);
            }
        }
        let mut suffix = 0u8;
        if trailing > 0 {
            if trailing >= 2 {
                r.skip(1)?;
                suffix = r.u8()?;
                r.skip(trailing - 2)?;
            } else {
                r.skip(trailing)?;
            }
        }
        out.push(RawParking {
            name,
            push_back,
            kind,
            number,
            radius_m,
            heading,
            lat,
            lon,
            airlines,
            suffix,
            tee_offsets,
        });
    }
    Ok((out, r.pos()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgl::testutil::{record, Bytes};

    fn slice(id: u16, body: &[u8]) -> (Vec<u8>, u16) {
        (record(id, body), id)
    }

    #[test]
    fn parses_taxi_points() {
        let body = Bytes::new()
            .u16(2)
            .u8(1)
            .u8(0)
            .zeros(2)
            .pos2(25.2528, 55.3644)
            .u8(2)
            .u8(1)
            .zeros(2)
            .pos2(25.2530, 55.3650)
            .done();
        let (bytes, id) = slice(AP_TAXI_POINT, &body);
        let rec = RecordSlice {
            id,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let pts = parse_taxi_points(&rec, &mut w).unwrap();
        assert!(w.is_empty());
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].kind, 1);
        assert_eq!(pts[1].kind, 2);
        assert_eq!(pts[1].orientation, 1);
        assert!((pts[0].lat - 25.2528).abs() < 1e-6);
        assert!((pts[0].lon - 55.3644).abs() < 1e-6);
    }

    #[test]
    fn parses_taxi_names() {
        let body = Bytes::new()
            .u16(3)
            .raw(b"\0\0\0\0\0\0\0\0")
            .raw(b"A\0\0\0\0\0\0\0")
            .raw(b"M12\0\0\0\0\0")
            .done();
        let (bytes, id) = slice(AP_TAXI_NAME, &body);
        let rec = RecordSlice {
            id,
            offset: 0,
            data: &bytes,
        };
        let names = parse_taxi_names(&rec).unwrap();
        assert_eq!(names, vec!["".to_string(), "A".to_string(), "M12".to_string()]);
    }

    fn taxi_path_element(start: u16, end: u16, kind: u8, msfs: bool) -> Bytes {
        let b = Bytes::new()
            .u16(start)
            .u16(if msfs { 0 } else { end })
            .u8(kind | (1 << 5)) // draw surface
            .u8(3) // name index
            .u8(0b1000_0111) // centerline + lit, left solid, right edge lit
            .u8(4) // asphalt
            .f32(23.0);
        if msfs {
            b.zeros(4 + 4 + 4).zeros(16).zeros(6).u16(end)
        } else {
            b.zeros(4).zeros(4)
        }
    }

    #[test]
    fn parses_fsx_taxi_paths() {
        let body = Bytes::new()
            .u16(2)
            .raw(&taxi_path_element(0, 1, 1, false).done())
            .raw(&taxi_path_element(1, 2, 2, false).done())
            .done();
        let (bytes, id) = slice(AP_TAXI_PATH, &body);
        let rec = RecordSlice {
            id,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let paths = parse_taxi_paths(&rec, &mut w).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(paths.len(), 2);
        assert_eq!((paths[0].start, paths[0].end), (0, 1));
        assert_eq!((paths[1].start, paths[1].end), (1, 2));
        assert_eq!(paths[0].kind, 1);
        assert!(paths[0].draw_surface);
        assert!(paths[0].centerline && paths[0].centerline_lit);
        assert_eq!(paths[0].left_edge, 1);
        assert!(paths[0].right_edge_lit);
        assert_eq!(paths[0].width_m, 23.0);
    }

    #[test]
    fn parses_msfs_taxi_paths_with_trailing_end_index() {
        let body = Bytes::new()
            .u16(2)
            .raw(&taxi_path_element(7, 4095 + 5, 1, true).done())
            .raw(&taxi_path_element(4100, 9, 2, true).done())
            .done();
        let (bytes, id) = slice(AP_TAXI_PATH_MSFS, &body);
        let rec = RecordSlice {
            id,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let paths = parse_taxi_paths(&rec, &mut w).unwrap();
        assert!(w.is_empty(), "{w:?}");
        // The MSFS end index is a full u16 and is not truncated to 12 bits.
        assert_eq!(paths[0].end, 4100);
        assert_eq!(paths[1].start, 4100);
        assert_eq!(paths[1].end, 9);
    }

    fn parking_element(number: u16, name: u8, kind: u8, airlines: &[&str], trailing: usize) -> Bytes {
        let flags = (name as u32 & 0x3F)
            | ((kind as u32 & 0xF) << 8)
            | ((number as u32 & 0xFFF) << 12)
            | ((airlines.len() as u32 & 0xFF) << 24);
        let mut b = Bytes::new()
            .u32(flags)
            .f32(30.0)
            .f32(180.0)
            .f32(0.0)
            .f32(0.0)
            .f32(0.0)
            .f32(0.0)
            .pos2(25.25, 55.36);
        for a in airlines {
            let mut buf = a.as_bytes().to_vec();
            buf.resize(4, 0);
            b = b.raw(&buf);
        }
        if trailing >= 2 {
            b = b.u8(0).u8(14).zeros(trailing - 2); // suffix code 14 == GATE_C
        } else {
            b = b.zeros(trailing);
        }
        b
    }

    #[test]
    fn parses_msfs_parkings_with_airlines_and_suffix() {
        let body = Bytes::new()
            .u16(2)
            .raw(&parking_element(12, 0x0C, 0x0A, &["UAE", "QTR"], 20).done())
            .raw(&parking_element(3, 0x01, 0x01, &[], 20).done())
            .done();
        let (bytes, id) = slice(AP_TAXI_PARKING_MSFS, &body);
        let rec = RecordSlice {
            id,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let p = parse_parkings(&rec, &mut w).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].number, 12);
        assert_eq!(p[0].name, 0x0C);
        assert_eq!(p[0].kind, 0x0A);
        assert_eq!(p[0].airlines, vec!["UAE", "QTR"]);
        assert_eq!(p[0].suffix, 14);
        assert_eq!(p[1].number, 3);
        assert!(p[1].airlines.is_empty());
    }

    #[test]
    fn parses_fsx_parkings() {
        let body = Bytes::new()
            .u16(1)
            .raw(&parking_element(5, 0x01, 0x02, &[], 0).done())
            .done();
        let (bytes, id) = slice(AP_TAXI_PARKING, &body);
        let rec = RecordSlice {
            id,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let p = parse_parkings(&rec, &mut w).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(p[0].number, 5);
        assert_eq!(p[0].radius_m, 30.0);
    }

    #[test]
    fn parkings_with_unknown_trailing_size_still_parse() {
        // Pretend a future build added 8 more trailing bytes per element.
        let body = Bytes::new()
            .u16(2)
            .raw(&parking_element(1, 0x01, 0x01, &[], 28).done())
            .raw(&parking_element(2, 0x01, 0x01, &[], 28).done())
            .done();
        let (bytes, id) = slice(AP_TAXI_PARKING_MSFS, &body);
        let rec = RecordSlice {
            id,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let p = parse_parkings(&rec, &mut w).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].number, 2);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn truncated_array_warns_and_returns_partial() {
        let mut body = Bytes::new()
            .u16(4)
            .u8(1)
            .u8(0)
            .zeros(2)
            .pos2(1.0, 2.0)
            .u8(1)
            .u8(0)
            .zeros(2)
            .pos2(1.0, 2.0)
            .done();
        body.truncate(body.len() - 3);
        let (bytes, id) = slice(AP_TAXI_POINT, &body);
        let rec = RecordSlice {
            id,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let pts = parse_taxi_points(&rec, &mut w).unwrap();
        assert!(pts.len() < 4);
        assert!(!w.is_empty());
    }
}
