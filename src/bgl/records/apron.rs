//! Aprons, apron edge lights and the legacy triangulated apron record.
//!
//! The MSFS apron record (`0xD0`, `0xD3`) puts a material GUID and several
//! unidentified fields before the vertex array, and MSFS 2024 changed their
//! sizes. The vertex array itself is found by scanning for decodable
//! coordinates, which is immune to that churn; the boundary is all we need,
//! because the trailing index/triangulation data has no X-Plane equivalent.

use crate::bgl::file::RecordSlice;
use crate::bgl::reader::BglError;

use crate::bgl::guid::Guid;

use super::geoscan::{count_matches, find_vertex_run, read_vertices, Near};
use super::ids::*;
use super::raw::RawApron;

/// Apron boundary record (`0x37`, `0xAF`, `0xD3`, `0xD0`).
pub fn parse_apron(rec: &RecordSlice, near: &Near, warnings: &mut Vec<String>) -> Result<RawApron, BglError> {
    let mut r = rec.body();
    let surface = r.u8()? & 0x7F;
    let data = rec.data;

    // MSFS layout: a flag byte at 6 (where older formats kept the surface),
    // an RGBA tint at 8..12 and the material GUID at 12..28.
    let flags = data.get(6).copied().unwrap_or(0);
    let tint = data.get(8..12).map(|t| [t[0], t[1], t[2], t[3]]).unwrap_or([0; 4]);
    let material = if rec.id == AP_APRON_FIRST_MSFS || rec.id == AP_APRON_FIRST_MSFS_NEW {
        data.get(12..28).and_then(Guid::from_slice).filter(|g| !g.is_nil())
    } else {
        None
    };

    let found = find_vertex_run(data, near, 3);
    let (at, count) = match found {
        Some(v) => v,
        None => {
            warnings.push(format!("apron 0x{:04X}: no vertex run found", rec.id));
            return Ok(RawApron {
                surface,
                draw_surface: true,
                draw_detail: true,
                vertices: Vec::new(),
                flags,
                tint,
                material,
            });
        }
    };
    if !count_matches(data, at, count) {
        // Not fatal: the run is still almost certainly the boundary, but say so
        // in case a future layout puts something else there.
        warnings.push(format!(
            "apron 0x{:04X}: vertex count field does not match the {count} vertices found",
            rec.id
        ));
    }
    Ok(RawApron {
        surface,
        draw_surface: true,
        draw_detail: true,
        vertices: read_vertices(data, at, count),
        flags,
        tint,
        material,
    })
}

/// Legacy triangulated apron (`0x30`, `0x41`, `0xB0`), FSX/P3D only.
/// Only the boundary is kept; the triangle list has no X-Plane equivalent.
pub fn parse_apron2(rec: &RecordSlice) -> Result<RawApron, BglError> {
    let mut r = rec.body();
    let surface = r.u8()? & 0x7F;
    let flags = r.u8()?;
    let head_extra = match rec.id {
        AP_APRON_SECOND_P3D_V5 => 20,
        AP_APRON_SECOND_P3D_V4 => 16,
        _ => 0,
    };
    r.skip(head_extra)?;
    let count = r.u16()? as usize;
    let _triangles = r.u16()?;
    Ok(RawApron {
        surface,
        draw_surface: flags & 1 != 0,
        draw_detail: flags & 2 != 0,
        vertices: read_vertices(rec.data, r.pos(), count),
        ..Default::default()
    })
}

/// Apron edge lights (`0x31`): polylines along which lights are drawn.
pub fn parse_apron_edge_lights(rec: &RecordSlice, near: &Near) -> Result<Vec<(f64, f64)>, BglError> {
    match find_vertex_run(rec.data, near, 2) {
        Some((at, count)) => Ok(read_vertices(rec.data, at, count)),
        None => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgl::testutil::{record, Bytes};

    const LAT: f64 = 25.2528;
    const LON: f64 = 55.3644;

    #[test]
    fn parses_msfs_apron_with_guid_and_unknown_fields() {
        // Mirrors the real MSFS 2020 layout: surface, GUID, assorted fields,
        // vertex count at offset 48, vertices at 52, index data afterwards.
        let body = Bytes::new()
            .u8(3) // flags
            .u8(0xFF)
            .raw(&[0x69, 0x6E, 0x72, 0xFF]) // tint
            .zeros(16) // material GUID: none
            .f32(25.0)
            .f32(0.0)
            .f32(2.0)
            .u32(1)
            .u32(0)
            .u16(4) // vertex count, at record offset 48
            .u16(2) // triangle count
            .pos2(LAT, LON)
            .pos2(LAT + 0.001, LON)
            .pos2(LAT + 0.001, LON + 0.001)
            .pos2(LAT, LON + 0.001)
            .u16(0)
            .u16(1)
            .u16(2)
            .u16(0)
            .u16(2)
            .u16(3)
            .done();
        let bytes = record(AP_APRON_FIRST_MSFS_NEW, &body);
        assert_eq!(
            u16::from_le_bytes([bytes[48], bytes[49]]),
            4,
            "count must sit at offset 48"
        );
        let rec = RecordSlice {
            id: AP_APRON_FIRST_MSFS_NEW,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let a = parse_apron(&rec, &Near::new(LAT, LON), &mut w).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(a.surface, 3);
        assert_eq!(a.flags, 3);
        assert_eq!(a.tint, [0x69, 0x6E, 0x72, 0xFF]);
        assert!(a.material.is_none(), "an all-zero GUID means no material");
        assert_eq!(a.vertices.len(), 4);
        assert!((a.vertices[0].0 - LAT).abs() < 1e-6);
        assert!((a.vertices[2].1 - (LON + 0.001)).abs() < 1e-6);
    }

    #[test]
    fn parses_fsx_apron() {
        let body = Bytes::new()
            .u8(0)
            .u16(3)
            .pos2(LAT, LON)
            .pos2(LAT + 0.001, LON)
            .pos2(LAT, LON + 0.001)
            .done();
        let bytes = record(AP_APRON_FIRST, &body);
        let rec = RecordSlice {
            id: AP_APRON_FIRST,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let a = parse_apron(&rec, &Near::new(LAT, LON), &mut w).unwrap();
        assert_eq!(a.vertices.len(), 3);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn parses_apron_edge_lights() {
        let body = Bytes::new()
            .u16(3)
            .pos2(LAT, LON)
            .pos2(LAT + 0.0005, LON)
            .pos2(LAT + 0.001, LON)
            .done();
        let bytes = record(AP_APRON_EDGE_LIGHTS, &body);
        let rec = RecordSlice {
            id: AP_APRON_EDGE_LIGHTS,
            offset: 0,
            data: &bytes,
        };
        let v = parse_apron_edge_lights(&rec, &Near::new(LAT, LON)).unwrap();
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn unparseable_apron_returns_empty_and_warns() {
        let bytes = record(AP_APRON_FIRST_MSFS, &[4, 0xFF, 0xFF, 0xFF]);
        let rec = RecordSlice {
            id: AP_APRON_FIRST_MSFS,
            offset: 0,
            data: &bytes,
        };
        let mut w = Vec::new();
        let a = parse_apron(&rec, &Near::new(LAT, LON), &mut w).unwrap();
        assert!(a.vertices.is_empty());
        assert!(!w.is_empty());
    }
}
