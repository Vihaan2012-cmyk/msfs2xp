//! The small fixed-layout records: name, COM, start, helipad, jetway.

use crate::bgl::codec::{alt_from_i32, lat_from_u32, lon_from_u32};
use crate::bgl::file::RecordSlice;
use crate::bgl::reader::BglError;

use super::raw::{RawCom, RawHelipad, RawJetway, RawStart};
use super::scenery::parse_library_object;

/// Airport / navaid name record: the rest of the record is the string.
pub fn parse_name(rec: &RecordSlice) -> Result<String, BglError> {
    let mut r = rec.body();
    let len = rec.data.len().saturating_sub(6);
    r.string_fixed(len)
}

/// COM frequency: `u16 type`, `u32 frequency in Hz`, 48-byte name.
pub fn parse_com(rec: &RecordSlice) -> Result<RawCom, BglError> {
    let mut r = rec.body();
    let kind = r.u16()?;
    let freq_hz = r.u32()?;
    let name_len = rec.data.len().saturating_sub(12).min(0x30);
    let name = r.string_fixed(name_len).unwrap_or_default();
    Ok(RawCom {
        kind,
        freq_khz: freq_hz / 1000,
        name: sanitize(&name),
    })
}

fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '/' || *c == '.')
        .collect::<String>()
        .trim()
        .to_string()
}

/// Start position: `u8 runway number`, `u8 designator|type`, position, heading.
pub fn parse_start(rec: &RecordSlice) -> Result<RawStart, BglError> {
    let mut r = rec.body();
    let number = r.u8()?;
    let flags = r.u8()?;
    let lon = lon_from_u32(r.u32()?);
    let lat = lat_from_u32(r.u32()?);
    let alt_m = alt_from_i32(r.i32()?);
    let heading = r.f32().unwrap_or(0.0);
    Ok(RawStart {
        number,
        designator: flags & 0x0F,
        kind: (flags >> 4) & 0x0F,
        lat,
        lon,
        alt_m,
        heading,
    })
}

/// Helipad: surface, type/flags, colour, position, length, width, heading.
pub fn parse_helipad(rec: &RecordSlice) -> Result<RawHelipad, BglError> {
    let mut r = rec.body();
    let surface = r.u8()? & 0x7F;
    let flags = r.u8()?;
    r.skip(4)?; // colour
    let lon = lon_from_u32(r.u32()?);
    let lat = lat_from_u32(r.u32()?);
    let alt_m = alt_from_i32(r.i32()?);
    let length_m = r.f32()?;
    let width_m = r.f32()?;
    let heading = r.f32().unwrap_or(0.0);
    Ok(RawHelipad {
        surface,
        kind: flags & 0x0F,
        transparent: flags & (1 << 4) != 0,
        closed: flags & (1 << 5) != 0,
        lat,
        lon,
        alt_m,
        length_m,
        width_m,
        heading,
    })
}

/// Jetway: the stand it serves and, in MSFS, where its model is placed.
///
/// Both generations open with the parking number and name code. The MSFS
/// record (0x00DE) follows them with a u32 length and a complete scenery
/// library-object record, which is where the jetway's position comes from.
pub fn parse_jetway(rec: &RecordSlice) -> Result<RawJetway, BglError> {
    let mut r = rec.body();
    let parking_number = r.u16()?;
    let parking_name = r.u16().unwrap_or(0);
    // Offset 0x12 is where every sample so far puts the embedded record; scan
    // nearby in case a later build moves it.
    let placement = std::iter::once(0x12usize)
        .chain((0x0Ausize..0x30).step_by(2))
        .find_map(|at| rec.data.get(at..).and_then(parse_library_object));
    Ok(RawJetway {
        parking_number,
        parking_name,
        placement,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgl::records::ids::*;
    use crate::bgl::testutil::{record, Bytes};

    #[test]
    fn parses_name() {
        let bytes = record(AP_NAME, "Dubai International".as_bytes());
        let rec = RecordSlice {
            id: AP_NAME,
            offset: 0,
            data: &bytes,
        };
        assert_eq!(parse_name(&rec).unwrap(), "Dubai International");
    }

    #[test]
    fn parses_com_frequency_in_khz() {
        let mut body = Bytes::new().u16(6).u32(118_400_000).done();
        let mut name = b"DUBAI TOWER".to_vec();
        name.resize(0x30, 0);
        body.extend_from_slice(&name);
        let bytes = record(AP_COM, &body);
        let rec = RecordSlice {
            id: AP_COM,
            offset: 0,
            data: &bytes,
        };
        let com = parse_com(&rec).unwrap();
        assert_eq!(com.kind, 6);
        assert_eq!(com.freq_khz, 118_400);
        assert_eq!(com.name, "DUBAI TOWER");
    }

    #[test]
    fn parses_start_type_and_designator() {
        let body = Bytes::new().u8(12).u8(0x31).pos(25.25, 55.36, 19.0).f32(120.0).done();
        let bytes = record(AP_START, &body);
        let rec = RecordSlice {
            id: AP_START,
            offset: 0,
            data: &bytes,
        };
        let s = parse_start(&rec).unwrap();
        assert_eq!(s.number, 12);
        assert_eq!(s.designator, 1); // L
        assert_eq!(s.kind, 3); // helipad
        assert_eq!(s.heading, 120.0);
    }

    #[test]
    fn parses_helipad_flags() {
        let body = Bytes::new()
            .u8(0) // concrete
            .u8(0x01 | (1 << 4)) // type H, transparent
            .zeros(4)
            .pos(25.25, 55.36, 19.0)
            .f32(20.0)
            .f32(20.0)
            .f32(45.0)
            .done();
        let bytes = record(AP_HELIPAD, &body);
        let rec = RecordSlice {
            id: AP_HELIPAD,
            offset: 0,
            data: &bytes,
        };
        let h = parse_helipad(&rec).unwrap();
        assert_eq!(h.kind, 1);
        assert!(h.transparent);
        assert!(!h.closed);
        assert_eq!(h.length_m, 20.0);
        assert_eq!(h.heading, 45.0);
    }

    #[test]
    fn parses_an_msfs_jetway_with_its_embedded_placement() {
        let guid = [
            0xA1u8, 0x32, 0xC2, 0xD1, 0x18, 0x85, 0xE4, 0x41, 0x88, 0xD7, 0x8F, 0x5E, 0x15, 0x8E, 0x35, 0xF3,
        ];
        let placement = Bytes::new()
            .u16(0x000B)
            .u16(64)
            .pos2(25.2486, 55.3601)
            .i32(0)
            .u16(1)
            .u16(0)
            .u16(0)
            .u16(0x4000)
            .zeros(4)
            .zeros(16)
            .raw(&guid)
            .f32(1.0)
            .done();
        let body = Bytes::new()
            .u16(18)
            .u16(0x0D)
            .u16(29)
            .u16(0)
            .u32(64)
            .raw(&placement)
            .done();
        let bytes = record(AP_MSFS_JETWAY, &body);
        let rec = RecordSlice {
            id: AP_MSFS_JETWAY,
            offset: 0,
            data: &bytes,
        };
        let j = parse_jetway(&rec).unwrap();
        assert_eq!((j.parking_number, j.parking_name), (18, 0x0D));
        let p = j.placement.expect("embedded placement");
        assert!((p.lat - 25.2486).abs() < 1e-6);
        assert!((p.heading - 90.0).abs() < 0.01);
        assert_eq!(p.guid.0, guid);
    }

    #[test]
    fn a_legacy_jetway_has_no_placement() {
        let bytes = record(AP_JETWAY, &Bytes::new().u16(4).u16(0x0C).done());
        let rec = RecordSlice {
            id: AP_JETWAY,
            offset: 0,
            data: &bytes,
        };
        let j = parse_jetway(&rec).unwrap();
        assert_eq!((j.parking_number, j.parking_name), (4, 0x0C));
        assert!(j.placement.is_none());
    }

    #[test]
    fn truncated_com_does_not_fail_hard() {
        let bytes = record(AP_COM, &Bytes::new().u16(6).u32(118_400_000).done());
        let rec = RecordSlice {
            id: AP_COM,
            offset: 0,
            data: &bytes,
        };
        let com = parse_com(&rec).unwrap();
        assert_eq!(com.freq_khz, 118_400);
        assert_eq!(com.name, "");
    }
}
