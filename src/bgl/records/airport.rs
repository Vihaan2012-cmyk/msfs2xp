//! The airport record: a fixed head followed by every other airport sub-record.

use crate::bgl::codec::{alt_from_i32, icao_from_u32, icao_from_u64, lat_from_u32, lon_from_u32, magvar_adjust};
use crate::bgl::file::{RecordSlice, SubRecords};
use crate::bgl::reader::BglError;

use super::geoscan::Near;
use super::ids::*;
use super::layout::probe_subrecord_start;
use super::raw::{RawAirport, Variant};
use super::{apron, msfs, runway, simple, taxi};

/// Head length (excluding the 6-byte record header) per generation.
const HEAD_FS9: usize = 46;
const HEAD_FSX: usize = 50;
const HEAD_P3DV5: usize = 58;
const HEAD_MSFS: usize = 62;

/// Parse one airport record. `hint` forces a layout family; `None` detects it.
pub fn parse_airport(rec: &RecordSlice, hint: Option<Variant>) -> Result<RawAirport, BglError> {
    let data = rec.data;
    let mut ap = RawAirport::default();
    let mut r = rec.body();

    r.skip(6)?; // runway/com/start/approach/apron/helipad counts
    ap.lon = lon_from_u32(r.u32()?);
    ap.lat = lat_from_u32(r.u32()?);
    ap.alt_m = alt_from_i32(r.i32()?);
    ap.tower_lon = lon_from_u32(r.u32()?);
    ap.tower_lat = lat_from_u32(r.u32()?);
    ap.tower_alt_m = alt_from_i32(r.i32()?);
    ap.magvar = magvar_adjust(r.f32()?);
    ap.ident = icao_from_u32(r.u32()?, true);
    ap.region = icao_from_u32(r.u32()?, true);
    ap.fuel_flags = r.u32()?;
    debug_assert_eq!(r.pos(), 6 + HEAD_FS9);

    // MSFS 2024 leaves the 32-bit ident empty and stores the 8-character ident
    // as a 64-bit value at record offset 0x4C, its low six bits used as flags.
    let is_2024 = rec.id == REC_AIRPORT_MSFS2024;
    if is_2024 {
        if let Some(raw) = data.get(0x4C..0x54) {
            let mut b = [0u8; 8];
            b.copy_from_slice(raw);
            let ident = icao_from_u64(u64::from_le_bytes(b), true);
            if !ident.is_empty() {
                ap.ident = ident;
            }
        }
    }

    // The MSFS head carries a closed flag that no other generation has.
    let msfs_flags = data.get(6 + HEAD_FS9 + 2).copied().unwrap_or(0);

    let candidates: Vec<usize> = match hint {
        Some(Variant::Fs9) => vec![6 + HEAD_FS9],
        Some(Variant::Fsx) | Some(Variant::P3dV4) => vec![6 + HEAD_FSX],
        Some(Variant::P3dV5) => vec![6 + HEAD_P3DV5],
        Some(Variant::Msfs2020) | Some(Variant::Msfs2024) => {
            let mut v = vec![6 + HEAD_MSFS];
            v.extend((4..=32).step_by(4).map(|e| 6 + HEAD_MSFS + e));
            v
        }
        _ => {
            let mut v = vec![6 + HEAD_MSFS, 6 + HEAD_FSX, 6 + HEAD_P3DV5, 6 + HEAD_FS9];
            v.extend((4..=32).step_by(4).map(|e| 6 + HEAD_MSFS + e));
            v
        }
    };

    let probe = probe_subrecord_start(data, &candidates, is_airport_subrecord);
    let start = match probe {
        Some(p) => p.start,
        None => {
            ap.warnings.push("airport: no readable sub-records".into());
            ap.variant = hint.unwrap_or(Variant::Unknown);
            return Ok(ap);
        }
    };

    ap.variant = hint.unwrap_or(match start {
        s if s == 6 + HEAD_FS9 => Variant::Fs9,
        s if s == 6 + HEAD_FSX => Variant::Fsx,
        s if s == 6 + HEAD_P3DV5 => Variant::P3dV5,
        _ => Variant::Msfs2020,
    });
    if is_2024 && hint.is_none() {
        ap.variant = Variant::Msfs2024;
    }
    if ap.variant.is_msfs() {
        ap.closed = msfs_flags & 0x04 != 0;
    }

    let msfs = ap.variant.is_msfs();
    let near = Near::new(ap.lat, ap.lon);

    for sub in SubRecords::new(data, start) {
        match sub.id {
            AP_NAME => ap.name = simple::parse_name(&sub).unwrap_or_default(),
            AP_TOWER_OBJ => ap.has_tower_obj = true,
            AP_RUNWAY | AP_RUNWAY_P3D_V4 | AP_RUNWAY_MSFS => {
                let is_msfs_rw = sub.id == AP_RUNWAY_MSFS || msfs;
                match runway::parse_runway(&sub, is_msfs_rw, &mut ap.warnings) {
                    Ok(rw) => ap.runways.push(rw),
                    Err(e) => ap.warnings.push(format!("runway: {e}")),
                }
            }
            AP_COM => {
                if let Ok(com) = simple::parse_com(&sub) {
                    if com.freq_khz > 0 {
                        ap.coms.push(com);
                    }
                }
            }
            AP_START => {
                if let Ok(s) = simple::parse_start(&sub) {
                    ap.starts.push(s);
                }
            }
            AP_HELIPAD => {
                if let Ok(h) = simple::parse_helipad(&sub) {
                    ap.helipads.push(h);
                }
            }
            AP_TAXI_POINT | AP_TAXI_POINT_P3D_V5 => match taxi::parse_taxi_points(&sub, &mut ap.warnings) {
                Ok(mut p) => ap.taxi_points.append(&mut p),
                Err(e) => ap.warnings.push(format!("taxi points: {e}")),
            },
            AP_TAXI_NAME => {
                if let Ok(mut n) = taxi::parse_taxi_names(&sub) {
                    ap.taxi_names.append(&mut n);
                }
            }
            AP_TAXI_PATH | AP_TAXI_PATH_P3D_V4 | AP_TAXI_PATH_P3D_V5 | AP_TAXI_PATH_MSFS => {
                match taxi::parse_taxi_paths(&sub, &mut ap.warnings) {
                    Ok(mut p) => ap.taxi_paths.append(&mut p),
                    Err(e) => ap.warnings.push(format!("taxi paths: {e}")),
                }
            }
            AP_TAXI_PARKING | AP_TAXI_PARKING_P3D_V5 | AP_TAXI_PARKING_MSFS | AP_TAXI_PARKING_FS9 => {
                match taxi::parse_parkings(&sub, &mut ap.warnings) {
                    Ok(mut p) => ap.parkings.append(&mut p),
                    Err(e) => ap.warnings.push(format!("parkings: {e}")),
                }
            }
            AP_APRON_FIRST | AP_APRON_FIRST_P3D_V5 | AP_APRON_FIRST_MSFS | AP_APRON_FIRST_MSFS_NEW => {
                if let Ok(a) = apron::parse_apron(&sub, &near, &mut ap.warnings) {
                    if a.vertices.len() >= 3 {
                        ap.aprons.push(a);
                    }
                }
            }
            AP_APRON_SECOND | AP_APRON_SECOND_P3D_V4 | AP_APRON_SECOND_P3D_V5 => {
                if let Ok(a) = apron::parse_apron2(&sub) {
                    if a.vertices.len() >= 3 && !msfs {
                        ap.aprons.push(a);
                    }
                }
            }
            AP_APRON_EDGE_LIGHTS => {
                if let Ok(v) = apron::parse_apron_edge_lights(&sub, &near) {
                    if v.vertices.len() >= 2 {
                        ap.apron_edge_lights.push(v);
                    }
                }
            }
            AP_JETWAY | AP_MSFS_JETWAY => {
                if let Ok(j) = simple::parse_jetway(&sub) {
                    ap.jetways.push(j);
                }
            }
            AP_MSFS_PAINTED_LINE => {
                if let Ok(l) = msfs::parse_painted_line(&sub, &near, &mut ap.warnings) {
                    if l.vertices.len() >= 2 {
                        ap.painted_lines.push(l);
                    }
                }
            }
            AP_MSFS_TAXIWAY_SIGN => {
                if let Ok(s) = msfs::parse_sign(&sub, &near, &mut ap.warnings) {
                    if !s.label.is_empty() {
                        ap.signs.push(s);
                    }
                }
            }
            AP_DELETE_AIRPORT | AP_DELETE_AIRPORT_NAV => ap.delete_airport = true,
            // Known but deliberately unused: procedures, meshes, fences, lighting helpers.
            AP_APPROACH
            | AP_APPROACH_MSFS
            | AP_MSFS_SID
            | AP_MSFS_STAR
            | AP_WAYPOINT
            | AP_FENCE_BLAST
            | AP_FENCE_BOUNDARY
            | AP_UNKNOWN_003B
            | AP_MSFS_LIGHT_SUPPORT
            | AP_MSFS_UNKNOWN_0058
            | AP_MSFS_UNKNOWN_0059
            | AP_MSFS_UNKNOWN_005A
            | AP_MSFS_UNKNOWN_005B
            | AP_MSFS_UNKNOWN_00CD
            | AP_MSFS_PAINTED_HATCHED_AREA
            | AP_MSFS_PARKING_MFGR_NAME
            | AP_MSFS_PROJECTED_MESH
            | AP_MSFS_GROUND_MERGING
            | AP_MSFS2024_MATERIAL_REF
            | AP_MSFS2024_UNKNOWN_005D
            | AP_MSFS2024_UNKNOWN_006A
            | AP_MSFS2024_UNKNOWN_00FB
            | AP_MSFS2024_UNKNOWN_00FF
            | AP_MSFS2024_WASM => {}
            other => ap.unknown_records.push((other, sub.size())),
        }
    }

    Ok(ap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgl::codec::icao_to_u32;
    use crate::bgl::testutil::{record, Bytes};

    const LAT: f64 = 25.2528;
    const LON: f64 = 55.3644;

    fn airport_head(ident: &str, extra: usize) -> Bytes {
        Bytes::new()
            .zeros(6)
            .pos(LAT, LON, 19.0)
            .pos(LAT + 0.001, LON + 0.001, 60.0)
            .f32(2.0)
            .u32(icao_to_u32(ident, true))
            .u32(0)
            .u32(0)
            .zeros(extra)
    }

    fn make(ident: &str, extra: usize, subs: &[Vec<u8>]) -> Vec<u8> {
        let mut body = airport_head(ident, extra).done();
        for s in subs {
            body.extend_from_slice(s);
        }
        record(REC_AIRPORT, &body)
    }

    #[test]
    fn parses_fsx_airport_head() {
        let name = record(AP_NAME, b"Test Field");
        let bytes = make("KTST", 4, &[name]);
        let rec = RecordSlice {
            id: REC_AIRPORT,
            offset: 0,
            data: &bytes,
        };
        let ap = parse_airport(&rec, None).unwrap();
        assert_eq!(ap.ident, "KTST");
        assert_eq!(ap.name, "Test Field");
        assert_eq!(ap.variant, Variant::Fsx);
        assert!((ap.lat - LAT).abs() < 1e-6);
        assert!((ap.alt_m - 19.0).abs() < 1e-3);
        assert_eq!(ap.magvar, -2.0);
    }

    #[test]
    fn parses_msfs_airport_head_and_closed_flag() {
        let mut head = Bytes::new()
            .zeros(6)
            .pos(LAT, LON, 19.0)
            .pos(LAT, LON, 60.0)
            .f32(0.0)
            .u32(icao_to_u32("OMDB", true))
            .u32(0)
            .u32(0)
            .done();
        // MSFS tail: 2 unknown, flags (closed), 1 unknown, then 12 more bytes.
        head.extend_from_slice(&[0, 0, 0x04, 0]);
        head.extend_from_slice(&[0u8; 12]);
        let mut body = head;
        body.extend_from_slice(&record(AP_NAME, b"Dubai Intl"));
        let bytes = record(REC_AIRPORT, &body);
        let rec = RecordSlice {
            id: REC_AIRPORT,
            offset: 0,
            data: &bytes,
        };
        let ap = parse_airport(&rec, None).unwrap();
        assert_eq!(ap.ident, "OMDB");
        assert_eq!(ap.variant, Variant::Msfs2020);
        assert!(ap.closed);
        assert_eq!(ap.name, "Dubai Intl");
    }

    #[test]
    fn collects_sub_records() {
        let name = record(AP_NAME, b"X");
        let com = record(AP_COM, &{
            let mut b = Bytes::new().u16(6).u32(118_100_000).done();
            b.resize(6 + 0x30, 0);
            b
        });
        let helipad = record(
            AP_HELIPAD,
            &Bytes::new()
                .u8(0)
                .u8(1)
                .zeros(4)
                .pos(LAT, LON, 19.0)
                .f32(20.0)
                .f32(20.0)
                .f32(0.0)
                .done(),
        );
        let bytes = make("KTST", 4, &[name, com, helipad]);
        let rec = RecordSlice {
            id: REC_AIRPORT,
            offset: 0,
            data: &bytes,
        };
        let ap = parse_airport(&rec, None).unwrap();
        assert_eq!(ap.coms.len(), 1);
        assert_eq!(ap.coms[0].freq_khz, 118_100);
        assert_eq!(ap.helipads.len(), 1);
    }

    #[test]
    fn unknown_subrecord_is_recorded_not_fatal() {
        let unknown = record(0x7777, &[1, 2, 3, 4]);
        let name = record(AP_NAME, b"Y");
        let bytes = make("KTST", 4, &[name, unknown]);
        let rec = RecordSlice {
            id: REC_AIRPORT,
            offset: 0,
            data: &bytes,
        };
        // The probe rejects offsets whose chain contains an invalid id, so an
        // unknown record at the end means the FSX offset no longer parses
        // exactly; parsing must still succeed and keep what it can.
        let ap = parse_airport(&rec, Some(Variant::Fsx)).unwrap();
        assert_eq!(ap.name, "Y");
        assert_eq!(ap.unknown_records, vec![(0x7777u16, 10usize)]);
    }

    #[test]
    fn layout_hint_is_respected() {
        // MSFS 2020 and 2024 share every record id, so the generation cannot be
        // derived from the bytes; it comes from the package manifest instead.
        let path_elem = Bytes::new()
            .u16(0)
            .u16(0)
            .u8(7)
            .u8(0)
            .u8(0)
            .u8(4)
            .f32(10.0)
            .zeros(12)
            .zeros(16)
            .zeros(6)
            .u16(1)
            .done();
        let paths = record(AP_TAXI_PATH_MSFS, &Bytes::new().u16(1).raw(&path_elem).done());
        let mut head = Bytes::new()
            .zeros(6)
            .pos(LAT, LON, 19.0)
            .pos(LAT, LON, 60.0)
            .f32(0.0)
            .u32(icao_to_u32("OMDB", true))
            .u32(0)
            .u32(0)
            .zeros(16)
            .done();
        head.extend_from_slice(&paths);
        let bytes = record(REC_AIRPORT, &head);
        let rec = RecordSlice {
            id: REC_AIRPORT,
            offset: 0,
            data: &bytes,
        };
        let ap = parse_airport(&rec, Some(Variant::Msfs2024)).unwrap();
        assert_eq!(ap.variant, Variant::Msfs2024);
        assert_eq!(ap.taxi_paths.len(), 1);
        assert_eq!(ap.taxi_paths[0].kind, 7);

        let ap = parse_airport(&rec, None).unwrap();
        assert_eq!(ap.variant, Variant::Msfs2020, "without a hint we assume 2020");
    }

    #[test]
    fn reads_the_msfs2024_record_and_its_64_bit_ident() {
        // The common 46-byte head with an empty 32-bit ident, then the 2024
        // extension: 24 bytes, the 64-bit ident at record offset 0x4C, 8 bytes.
        let ident64 = ((icao_to_u32("OMDB", false) as u64) << 6) | 1;
        let mut body = Bytes::new()
            .zeros(6)
            .pos(LAT, LON, 8.7)
            .zeros(12)
            .f32(357.7)
            .u32(0)
            .u32(0)
            .u32(0)
            .zeros(24)
            .done();
        body.extend_from_slice(&ident64.to_le_bytes());
        body.extend_from_slice(&[0u8; 8]);
        assert_eq!(body.len(), 86);
        body.extend_from_slice(&record(AP_NAME, b"Dubai Intl"));
        let bytes = record(REC_AIRPORT_MSFS2024, &body);
        assert_eq!(u64::from_le_bytes(bytes[0x4C..0x54].try_into().unwrap()), ident64);
        let rec = RecordSlice {
            id: REC_AIRPORT_MSFS2024,
            offset: 0,
            data: &bytes,
        };
        let ap = parse_airport(&rec, None).unwrap();
        assert_eq!(ap.ident, "OMDB");
        assert_eq!(ap.name, "Dubai Intl");
        assert_eq!(ap.variant, Variant::Msfs2024);
        assert!(ap.warnings.is_empty(), "{:?}", ap.warnings);
    }

    #[test]
    fn empty_record_does_not_panic() {
        let bytes = record(REC_AIRPORT, &[0u8; 10]);
        let rec = RecordSlice {
            id: REC_AIRPORT,
            offset: 0,
            data: &bytes,
        };
        assert!(parse_airport(&rec, None).is_err() || parse_airport(&rec, None).is_ok());
    }
}
