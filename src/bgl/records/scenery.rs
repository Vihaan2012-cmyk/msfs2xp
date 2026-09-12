//! Scenery object placements (section 0x25).
//!
//! Every building, sign, vehicle and tree in a package is a record here naming
//! a model by GUID, with a position and an orientation. Two things differ from
//! airport records: the size field is 16 bits, and there are two layouts of the
//! library-object record in circulation.
//!
//! - 64 bytes: the classic form. `u32` fixed-point position, `u16` angles.
//! - 92 bytes: the same head plus 28 bytes carrying a double-precision latitude
//!   and longitude and a 32-bit heading. Built by newer MSFS tooling.
//!
//! In both, the model GUID is the 16 bytes that end four bytes before the end of
//! the record, and the last four bytes are the scale. Anchoring on the end
//! rather than a fixed offset is what lets one parser read both.

use std::collections::BTreeMap;

use crate::bgl::codec::{alt_from_i32, lat_from_u32, lon_from_u32};
use crate::bgl::file::{BglFile, SECTION_SCENERY_OBJECT};
use crate::bgl::guid::Guid;

pub const SO_LIBRARY_OBJECT: u16 = 0x000B;
const LIB_OBJECT_MIN: usize = 64;
const LIB_OBJECT_HIRES: usize = 92;

/// One placed library object.
#[derive(Debug, Clone, PartialEq)]
pub struct RawPlacement {
    pub lat: f64,
    pub lon: f64,
    pub alt_m: f64,
    /// Altitude is relative to the ground rather than to sea level.
    pub agl: bool,
    pub pitch: f32,
    pub bank: f32,
    /// Degrees true.
    pub heading: f32,
    pub scale: f32,
    pub guid: Guid,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn f64_at(b: &[u8], at: usize) -> f64 {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&b[at..at + 8]);
    f64::from_le_bytes(raw)
}

/// A 16-bit fraction of a full turn, in degrees.
fn angle_u16(v: u16) -> f32 {
    v as f32 * 360.0 / 65_536.0
}

/// Fold an angle into (-180, 180] so that "359.99" pitch reads as level.
fn signed(a: f32) -> f32 {
    if a > 180.0 {
        a - 360.0
    } else {
        a
    }
}

/// Parse one library-object record, header included. Returns `None` for any
/// other record type or a record too short to hold a GUID.
pub fn parse_library_object(rec: &[u8]) -> Option<RawPlacement> {
    if rec.len() < LIB_OBJECT_MIN || u16_at(rec, 0) != SO_LIBRARY_OBJECT {
        return None;
    }
    let size = (u16_at(rec, 2) as usize).min(rec.len());
    if size < LIB_OBJECT_MIN {
        return None;
    }

    let mut lon = lon_from_u32(u32_at(rec, 0x04));
    let mut lat = lat_from_u32(u32_at(rec, 0x08));
    let alt_m = alt_from_i32(u32_at(rec, 0x0C) as i32);
    let flags = u16_at(rec, 0x10);
    let pitch = signed(angle_u16(u16_at(rec, 0x12)));
    let bank = signed(angle_u16(u16_at(rec, 0x14)));
    let coarse_heading = u16_at(rec, 0x16);
    let mut heading = angle_u16(coarse_heading);

    if size >= LIB_OBJECT_HIRES {
        // Only trust the precise copy when it agrees with the coarse one; that
        // guards against a future layout reusing these bytes for something else.
        let (plat, plon) = (f64_at(rec, 0x2C), f64_at(rec, 0x34));
        if plat.is_finite() && plon.is_finite() && (plat - lat).abs() < 1e-3 && (plon - lon).abs() < 1e-3 {
            lat = plat;
            lon = plon;
        }
        let fine = u32_at(rec, 0x44);
        if (fine >> 16) as u16 == coarse_heading {
            heading = (fine as f64 * 360.0 / 4_294_967_296.0) as f32;
        }
    }

    let guid = Guid::from_slice(&rec[size - 20..size - 4])?;
    let scale = f32::from_le_bytes([rec[size - 4], rec[size - 3], rec[size - 2], rec[size - 1]]);
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };

    Some(RawPlacement {
        lat,
        lon,
        alt_m,
        // Bit 0 of the flags is "is above AGL", as it has been since FSX.
        agl: flags & 0x0001 != 0,
        pitch,
        bank,
        heading: if heading >= 360.0 { heading - 360.0 } else { heading },
        scale,
        guid,
    })
}

/// Record id of a SimObject placement, which names its object by title.
pub const SO_SIM_OBJECT: u16 = 0x001D;

/// Parse a SimObject placement (0x001D).
///
/// It shares the library object's head (position, altitude, flags, angles)
/// but names a SimObject by title instead of a model by GUID, so the GUID is
/// left nil. MSFS 2024 jetways at some airports are placed this way.
pub fn parse_sim_object(rec: &[u8]) -> Option<RawPlacement> {
    if rec.len() < 0x18 || u16_at(rec, 0) != SO_SIM_OBJECT {
        return None;
    }
    if (u16_at(rec, 2) as usize) < 0x18 {
        return None;
    }
    Some(RawPlacement {
        lat: lat_from_u32(u32_at(rec, 0x08)),
        lon: lon_from_u32(u32_at(rec, 0x04)),
        alt_m: alt_from_i32(u32_at(rec, 0x0C) as i32),
        agl: u16_at(rec, 0x10) & 0x0001 != 0,
        pitch: signed(angle_u16(u16_at(rec, 0x12))),
        bank: signed(angle_u16(u16_at(rec, 0x14))),
        heading: angle_u16(u16_at(rec, 0x16)),
        scale: 1.0,
        guid: Guid::NIL,
    })
}

/// Everything found in a file's scenery-object sections.
#[derive(Debug, Default)]
pub struct PlacementScan {
    pub placements: Vec<RawPlacement>,
    /// Record types we saw but do not convert, with counts.
    pub other_kinds: BTreeMap<u16, usize>,
    /// `(lat, lon)` of windsock objects (record 0x0018), which X-Plane draws
    /// from its own apt.dat row.
    pub windsocks: Vec<(f64, f64)>,
}

/// Record id of a placed windsock.
pub const SO_WINDSOCK: u16 = 0x0018;

/// Collect every library-object placement in `file`.
pub fn parse_placements(file: &BglFile) -> PlacementScan {
    let data = file.data;
    let mut scan = PlacementScan::default();
    for section in file.sections.iter().filter(|s| s.kind == SECTION_SCENERY_OBJECT) {
        for sub in &section.subsections {
            let mut pos = sub.offset as usize;
            let end = pos.saturating_add(sub.size as usize).min(data.len());
            let mut seen = 0u32;
            while pos + 4 <= end && (sub.record_count == 0 || seen < sub.record_count) {
                let id = u16_at(data, pos);
                let size = u16_at(data, pos + 2) as usize;
                if size < 4 || pos + size > end {
                    break;
                }
                let rec = &data[pos..pos + size];
                if id == SO_WINDSOCK && size >= 12 {
                    scan.windsocks
                        .push((lat_from_u32(u32_at(rec, 0x08)), lon_from_u32(u32_at(rec, 0x04))));
                    pos += size;
                    seen += 1;
                    continue;
                }
                match parse_library_object(rec) {
                    Some(p) if id == SO_LIBRARY_OBJECT => scan.placements.push(p),
                    _ => *scan.other_kinds.entry(id).or_default() += 1,
                }
                pos += size;
                seen += 1;
            }
        }
    }
    scan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgl::testutil::{BglBuilder, Bytes};

    const GUID: [u8; 16] = [
        0xEE, 0x03, 0xC0, 0xCB, 0x45, 0x91, 0x1A, 0x49, 0x8A, 0x63, 0xC2, 0x8D, 0xEF, 0xCE, 0x68, 0x83,
    ];

    fn head(size: u16, lat: f64, lon: f64, heading_u16: u16) -> Bytes {
        Bytes::new()
            .u16(SO_LIBRARY_OBJECT)
            .u16(size)
            .pos2(lat, lon)
            .i32(0)
            .u16(0x0001) // above AGL
            .u16(0) // pitch
            .u16(0xFFFF) // bank: one step short of a full turn, i.e. level
            .u16(heading_u16)
            .u16(0x0100)
            .u16(0)
            .zeros(16) // instance id
    }

    fn classic(lat: f64, lon: f64, heading_u16: u16, scale: f32) -> Vec<u8> {
        head(64, lat, lon, heading_u16).raw(&GUID).f32(scale).done()
    }

    fn hires(lat: f64, lon: f64, fine_heading: u32, scale: f32) -> Vec<u8> {
        let mut b = head(92, lat, lon, (fine_heading >> 16) as u16).done();
        b.extend_from_slice(&lat.to_le_bytes());
        b.extend_from_slice(&lon.to_le_bytes());
        b.extend_from_slice(&[0u8; 8]);
        b.extend_from_slice(&fine_heading.to_le_bytes());
        b.extend_from_slice(&GUID);
        b.extend_from_slice(&scale.to_le_bytes());
        assert_eq!(b.len(), 92);
        b
    }

    #[test]
    fn reads_the_classic_64_byte_record() {
        let rec = classic(41.9786, -87.9048, 0x4000, 0.5);
        assert_eq!(rec.len(), 64);
        let p = parse_library_object(&rec).unwrap();
        assert!((p.lat - 41.9786).abs() < 1e-6);
        assert!((p.lon + 87.9048).abs() < 1e-6);
        assert!((p.heading - 90.0).abs() < 0.01);
        assert_eq!(p.scale, 0.5);
        assert_eq!(p.guid, Guid(GUID));
        assert!(p.agl);
        assert!(p.bank.abs() < 0.01, "0xFFFF bank should read as level, got {}", p.bank);
    }

    #[test]
    fn reads_the_high_precision_92_byte_record() {
        // A heading between two 16-bit steps proves the 32-bit value was used.
        let fine = 0x96A9_16FC;
        let rec = hires(25.235_451_123, 55.393_350_456, fine, 1.0);
        let p = parse_library_object(&rec).unwrap();
        assert_eq!(p.lat, 25.235_451_123, "double precision latitude is used");
        assert_eq!(p.lon, 55.393_350_456);
        assert!((p.heading - 211.866).abs() < 0.01, "heading {}", p.heading);
        assert_eq!(p.guid, Guid(GUID));
    }

    #[test]
    fn ignores_a_precise_position_that_disagrees() {
        let mut rec = hires(25.2354, 55.3933, 0x1000_0000, 1.0);
        rec[0x2C..0x34].copy_from_slice(&(-12.0f64).to_le_bytes());
        let p = parse_library_object(&rec).unwrap();
        assert!((p.lat - 25.2354).abs() < 1e-6, "coarse position kept");
    }

    #[test]
    fn reads_a_sim_object_placement() {
        let mut rec = head(72, 41.9739, -87.8867, 0xAC81).done();
        rec[0] = 0x1D;
        rec.extend_from_slice(&[0u8; 72 - 44]);
        let p = parse_sim_object(&rec).unwrap();
        assert!((p.lat - 41.9739).abs() < 1e-6);
        assert!((p.heading - 242.58).abs() < 0.01, "{}", p.heading);
        assert!(p.guid.is_nil());
        assert!(parse_library_object(&rec).is_none());
    }

    #[test]
    fn a_bad_scale_becomes_one() {
        let p = parse_library_object(&classic(1.0, 2.0, 0, f32::NAN)).unwrap();
        assert_eq!(p.scale, 1.0);
        let p = parse_library_object(&classic(1.0, 2.0, 0, -3.0)).unwrap();
        assert_eq!(p.scale, 1.0);
    }

    #[test]
    fn rejects_other_types_and_short_records() {
        let mut rec = classic(1.0, 2.0, 0, 1.0);
        rec[0] = 0x1B;
        assert!(parse_library_object(&rec).is_none());
        assert!(parse_library_object(&classic(1.0, 2.0, 0, 1.0)[..40]).is_none());
    }

    #[test]
    fn scans_a_section_and_counts_other_kinds() {
        let mut other = classic(1.0, 2.0, 0, 1.0);
        other[0] = 0x1B; // an unrelated record type of the same size
        let data = BglBuilder::new()
            .section(
                SECTION_SCENERY_OBJECT,
                vec![
                    classic(41.97, -87.90, 0, 1.0),
                    other,
                    hires(41.98, -87.91, 0x2000_0000, 2.0),
                ],
            )
            .build();
        let file = BglFile::parse(&data).unwrap();
        let scan = parse_placements(&file);
        assert_eq!(scan.placements.len(), 2);
        assert_eq!(scan.placements[1].scale, 2.0);
        assert_eq!(scan.other_kinds.get(&0x1B), Some(&1));
    }

    #[test]
    fn a_truncated_section_stops_cleanly() {
        let mut data = BglBuilder::new()
            .section(
                SECTION_SCENERY_OBJECT,
                vec![classic(1.0, 2.0, 0, 1.0), classic(1.0, 2.0, 0, 1.0)],
            )
            .build();
        data.truncate(data.len() - 10);
        let file = BglFile::parse(&data).unwrap();
        assert_eq!(parse_placements(&file).placements.len(), 1);
    }
}
