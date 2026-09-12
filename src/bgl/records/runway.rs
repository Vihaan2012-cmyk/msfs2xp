//! Runway records and their sub-records (thresholds, blast pads, VASI, approach lights).

use crate::bgl::codec::{alt_from_i32, icao_from_u32, lat_from_u32, lon_from_u32};
use crate::bgl::file::{RecordSlice, SubRecords};
use crate::bgl::reader::BglError;

use super::ids::*;
use super::layout::probe_subrecord_start;
use super::raw::{RawApproachLights, RawRunway, RawVasi};

/// Surface codes keep only the low seven bits; the top bit is a "hard" flag.
pub const SURFACE_MASK: u16 = 0x7F;

/// Fixed head length (excluding the 6-byte record header) for each generation.
const HEAD_FSX: usize = 46;
const HEAD_P3D: usize = HEAD_FSX + 16;
/// MSFS adds 24 bytes of unknowns, a 16-byte material GUID and 4 more bytes.
const HEAD_MSFS: usize = HEAD_FSX + 44;

/// Parse a runway record. `msfs` selects the sub-record layout for the
/// threshold/blast-pad records, which carry a material GUID in MSFS.
pub fn parse_runway(rec: &RecordSlice, msfs: bool, warnings: &mut Vec<String>) -> Result<RawRunway, BglError> {
    let data = rec.data;
    let mut r = rec.body();

    let mut rw = RawRunway {
        surface: (r.u16()? & SURFACE_MASK) as u8,
        ..Default::default()
    };
    rw.primary.number = r.u8()?;
    rw.primary.designator = r.u8()?;
    rw.secondary.number = r.u8()?;
    rw.secondary.designator = r.u8()?;
    rw.primary.ils_ident = icao_from_u32(r.u32()?, false);
    rw.secondary.ils_ident = icao_from_u32(r.u32()?, false);
    rw.lon = lon_from_u32(r.u32()?);
    rw.lat = lat_from_u32(r.u32()?);
    rw.alt_m = alt_from_i32(r.i32()?);
    rw.length_m = r.f32()?;
    rw.width_m = r.f32()?;
    rw.heading_true = r.f32()?;
    rw.pattern_alt_m = r.f32()?;
    rw.marking_flags = r.u16()? as u32;
    rw.light_flags = r.u8()?;
    rw.pattern_flags = r.u8()?;
    debug_assert_eq!(r.pos(), 6 + HEAD_FSX);

    // Where do the sub-records start? Candidates cover FSX, P3D, MSFS 2020 and
    // any future head that is a multiple of four bytes longer.
    let mut candidates = vec![6 + HEAD_FSX, 6 + HEAD_P3D, 6 + HEAD_MSFS];
    for extra in (4..=64).step_by(4) {
        candidates.push(6 + HEAD_MSFS + extra);
    }
    let start = match probe_subrecord_start(data, &candidates, is_runway_subrecord) {
        Some(p) => {
            if !p.exact {
                warnings.push(format!(
                    "runway {}/{}: sub-records do not fill the record exactly",
                    super::super::codec::runway_name(rw.primary.number, rw.primary.designator),
                    super::super::codec::runway_name(rw.secondary.number, rw.secondary.designator)
                ));
            }
            p.start
        }
        None => data.len(),
    };

    for sub in SubRecords::new(data, start) {
        match sub.id {
            RW_OFFSET_THRESHOLD_PRIM => rw.primary.offset_threshold_m = ext_length(&sub, msfs)?,
            RW_OFFSET_THRESHOLD_SEC => rw.secondary.offset_threshold_m = ext_length(&sub, msfs)?,
            RW_BLAST_PAD_PRIM => rw.primary.blast_pad_m = ext_length(&sub, msfs)?,
            RW_BLAST_PAD_SEC => rw.secondary.blast_pad_m = ext_length(&sub, msfs)?,
            RW_OVERRUN_PRIM => rw.primary.overrun_m = ext_length(&sub, msfs)?,
            RW_OVERRUN_SEC => rw.secondary.overrun_m = ext_length(&sub, msfs)?,
            // The newer MSFS overrun records carry no material GUID.
            RW_OVERRUN_PRIM_MSFS => rw.primary.overrun_m = ext_length(&sub, false)?,
            RW_OVERRUN_SEC_MSFS => rw.secondary.overrun_m = ext_length(&sub, false)?,
            RW_VASI_PRIM_LEFT => rw.primary.vasi_left = parse_vasi(&sub).ok(),
            RW_VASI_PRIM_RIGHT => rw.primary.vasi_right = parse_vasi(&sub).ok(),
            RW_VASI_SEC_LEFT => rw.secondary.vasi_left = parse_vasi(&sub).ok(),
            RW_VASI_SEC_RIGHT => rw.secondary.vasi_right = parse_vasi(&sub).ok(),
            RW_APP_LIGHTS_PRIM | RW_APP_LIGHTS_PRIM_MSFS => rw.primary.approach_lights = parse_app_lights(&sub).ok(),
            RW_APP_LIGHTS_SEC | RW_APP_LIGHTS_SEC_MSFS => rw.secondary.approach_lights = parse_app_lights(&sub).ok(),
            _ => {}
        }
    }
    Ok(rw)
}

/// Threshold / blast pad / overrun: surface, optional material GUID, length, width.
fn ext_length(rec: &RecordSlice, msfs: bool) -> Result<f32, BglError> {
    let mut r = rec.body();
    r.u16()?; // surface, same as the runway
    if msfs {
        r.skip(16)?; // material GUID
    }
    let len = r.f32()?;
    Ok(if len.is_finite() && len >= 0.0 { len } else { 0.0 })
}

fn parse_vasi(rec: &RecordSlice) -> Result<RawVasi, BglError> {
    let mut r = rec.body();
    let kind = r.u16()?;
    let bias_x = r.f32().unwrap_or(0.0);
    let bias_z = r.f32().unwrap_or(0.0);
    let spacing = r.f32().unwrap_or(0.0);
    let pitch = r.f32().unwrap_or(0.0);
    Ok(RawVasi {
        kind,
        bias_x,
        bias_z,
        spacing,
        pitch,
    })
}

fn parse_app_lights(rec: &RecordSlice) -> Result<RawApproachLights, BglError> {
    let mut r = rec.body();
    let flags = r.u8()?;
    let strobes = r.u8().unwrap_or(0);
    Ok(RawApproachLights {
        system: flags & 0x1F,
        end_lights: flags & 0x20 != 0,
        reil: flags & 0x40 != 0,
        touchdown: flags & 0x80 != 0,
        strobes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgl::testutil::{record, Bytes};

    fn runway_head(len: f32, width: f32, heading: f32, extra: usize) -> Bytes {
        Bytes::new()
            .u16(4) // asphalt
            .u8(9)
            .u8(1) // 09L
            .u8(27)
            .u8(2) // 27R
            .u32(0)
            .u32(0)
            .pos(40.6398, -73.7789, 4.0)
            .f32(len)
            .f32(width)
            .f32(heading)
            .f32(304.8)
            .u16(0b0000_0000_0110_0011) // edges|threshold|ident|precision
            .u8(0b0000_1011) // edge HIGH(3), center MEDIUM(2)
            .u8(0)
            .zeros(extra)
    }

    #[test]
    fn parses_fsx_runway_without_subrecords() {
        let body = runway_head(3000.0, 45.0, 90.0, 0).done();
        let bytes = record(AP_RUNWAY, &body);
        let rec = RecordSlice {
            id: AP_RUNWAY,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let rw = parse_runway(&rec, false, &mut w).unwrap();
        assert_eq!(rw.surface, 4);
        assert_eq!(rw.primary.number, 9);
        assert_eq!(rw.primary.designator, 1);
        assert_eq!(rw.secondary.number, 27);
        assert_eq!(rw.length_m, 3000.0);
        assert_eq!(rw.width_m, 45.0);
        assert_eq!(rw.heading_true, 90.0);
        assert!((rw.lat - 40.6398).abs() < 1e-6);
        assert_eq!(rw.light_flags & 0x3, 3);
    }

    #[test]
    fn parses_msfs_runway_with_subrecords() {
        let mut body = runway_head(4000.0, 60.0, 130.0, 44).done();
        // Displaced threshold on the primary end: surface, GUID, length, width.
        let thr = Bytes::new().u16(4).zeros(16).f32(300.0).f32(60.0).done();
        body.extend_from_slice(&record(RW_OFFSET_THRESHOLD_PRIM, &thr));
        // PAPI on the primary right side.
        let vasi = Bytes::new().u16(8).f32(0.0).f32(0.0).f32(0.0).f32(3.0).done();
        body.extend_from_slice(&record(RW_VASI_PRIM_RIGHT, &vasi));
        // ALSF-2 with touchdown zone lights and REIL.
        let al = Bytes::new().u8(0x07 | 0x40 | 0x80).u8(0).done();
        body.extend_from_slice(&record(RW_APP_LIGHTS_PRIM_MSFS, &al));
        let bytes = record(AP_RUNWAY_MSFS, &body);
        let rec = RecordSlice {
            id: AP_RUNWAY_MSFS,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let rw = parse_runway(&rec, true, &mut w).unwrap();
        assert!(w.is_empty(), "unexpected warnings: {w:?}");
        assert_eq!(rw.length_m, 4000.0);
        assert_eq!(rw.primary.offset_threshold_m, 300.0);
        let v = rw.primary.vasi_right.unwrap();
        assert_eq!(v.kind, 8);
        assert_eq!(v.pitch, 3.0);
        let al = rw.primary.approach_lights.unwrap();
        assert_eq!(al.system, 7);
        assert!(al.reil && al.touchdown && !al.end_lights);
    }

    #[test]
    fn probe_handles_longer_msfs2024_head() {
        // Pretend a future build added 12 more bytes before the sub-records.
        let mut body = runway_head(2500.0, 30.0, 10.0, 44 + 12).done();
        let ov = Bytes::new().u16(4).f32(60.0).f32(30.0).done();
        body.extend_from_slice(&record(RW_OVERRUN_SEC_MSFS, &ov));
        let bytes = record(AP_RUNWAY_MSFS, &body);
        let rec = RecordSlice {
            id: AP_RUNWAY_MSFS,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let rw = parse_runway(&rec, true, &mut w).unwrap();
        assert_eq!(rw.secondary.overrun_m, 60.0);
        assert!(w.is_empty());
    }

    #[test]
    fn garbage_tail_does_not_panic() {
        let mut body = runway_head(1000.0, 20.0, 0.0, 44).done();
        body.extend_from_slice(&[0xFF; 17]);
        let bytes = record(AP_RUNWAY_MSFS, &body);
        let rec = RecordSlice {
            id: AP_RUNWAY_MSFS,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let rw = parse_runway(&rec, true, &mut w).unwrap();
        assert_eq!(rw.length_m, 1000.0);
    }
}
