//! Locating coordinate payloads inside records whose layout is undocumented.
//!
//! A BGL coordinate pair is eight bytes of fixed-point longitude and latitude.
//! Decoding eight arbitrary bytes almost never yields a point within a few
//! kilometres of a given airport, so "does this decode near the airport?" is a
//! reliable way to find the vertex array in a record we do not otherwise
//! understand — and it keeps working when Asobo inserts new fields in front of
//! it, which is exactly what happened between MSFS 2020 and 2024.

use crate::bgl::codec::{lat_from_u32, lon_from_u32};

/// Airport reference point used to sanity-check decoded coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Near {
    pub lat: f64,
    pub lon: f64,
    /// Accept latitudes within this many degrees of the reference.
    pub tol_deg: f64,
}

impl Near {
    /// 0.2 degrees is roughly 22 km: larger than any airport, small enough that
    /// a false positive needs eight specific bytes in a row.
    pub fn new(lat: f64, lon: f64) -> Self {
        Near { lat, lon, tol_deg: 0.2 }
    }

    pub fn with_tolerance(lat: f64, lon: f64, tol_deg: f64) -> Self {
        Near { lat, lon, tol_deg }
    }

    pub fn accepts(&self, lat: f64, lon: f64) -> bool {
        if !lat.is_finite() || !lon.is_finite() {
            return false;
        }
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return false;
        }
        let dlat = (lat - self.lat).abs();
        let mut dlon = (lon - self.lon).abs();
        if dlon > 180.0 {
            dlon = 360.0 - dlon;
        }
        // Widen the longitude window towards the poles where degrees shrink.
        let scale = self.lat.to_radians().cos().abs().max(0.05);
        dlat <= self.tol_deg && dlon <= self.tol_deg / scale
    }
}

/// Decode the coordinate pair at `at`, if it is in range.
pub fn read_pair(data: &[u8], at: usize) -> Option<(f64, f64)> {
    if at + 8 > data.len() {
        return None;
    }
    let lon = lon_from_u32(u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]));
    let lat = lat_from_u32(u32::from_le_bytes([
        data[at + 4],
        data[at + 5],
        data[at + 6],
        data[at + 7],
    ]));
    Some((lat, lon))
}

/// How many consecutive acceptable coordinate pairs start at `at`.
pub fn run_length(data: &[u8], at: usize, near: &Near) -> usize {
    let mut n = 0;
    let mut p = at;
    while let Some((lat, lon)) = read_pair(data, p) {
        if !near.accepts(lat, lon) {
            break;
        }
        n += 1;
        p += 8;
    }
    n
}

/// The longest run of coordinate pairs in the record, as `(offset, count)`.
///
/// `min_count` rejects incidental matches. The scan is byte-wise because vertex
/// arrays are not reliably aligned: the FSX apron record puts a single-byte
/// surface field first, which leaves its vertices on an odd offset.
pub fn find_vertex_run(data: &[u8], near: &Near, min_count: usize) -> Option<(usize, usize)> {
    let step = 1usize;
    let mut best: Option<(usize, usize)> = None;
    let mut at = 6;
    while at + 8 <= data.len() {
        let n = run_length(data, at, near);
        if n >= min_count && best.map(|(_, bn)| n > bn).unwrap_or(true) {
            best = Some((at, n));
        }
        if n > 0 {
            at += (n * 8).max(step);
        } else {
            at += step;
        }
    }
    best
}

/// Read `count` coordinate pairs from `at` as `(lat, lon)`.
pub fn read_vertices(data: &[u8], at: usize, count: usize) -> Vec<(f64, f64)> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        match read_pair(data, at + i * 8) {
            Some(v) => out.push(v),
            None => break,
        }
    }
    out
}

/// Check whether a `u16` shortly before a vertex run equals its length.
///
/// Array records store the element count just before the data, but not always
/// immediately: the MSFS apron record puts a second count (triangles) between
/// them, so look back two and four bytes.
pub fn count_matches(data: &[u8], run_at: usize, run_len: usize) -> bool {
    [2usize, 4]
        .iter()
        .filter(|&&back| run_at >= back)
        .any(|&back| u16::from_le_bytes([data[run_at - back], data[run_at - back + 1]]) as usize == run_len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgl::testutil::Bytes;

    const LAT: f64 = 40.6414;
    const LON: f64 = -73.7777;

    #[test]
    fn finds_the_longest_run() {
        let body = Bytes::new()
            .zeros(6)
            .u32(0xDEAD_BEEF)
            .pos2(LAT, LON)
            .pos2(LAT + 0.001, LON)
            .zeros(4)
            .u16(3)
            .pos2(LAT, LON)
            .pos2(LAT, LON + 0.001)
            .pos2(LAT + 0.002, LON)
            .done();
        let near = Near::new(LAT, LON);
        let (at, n) = find_vertex_run(&body, &near, 2).unwrap();
        assert_eq!(n, 3);
        assert!(count_matches(&body, at, n));
    }

    #[test]
    fn rejects_short_runs() {
        let body = Bytes::new().zeros(6).pos2(LAT, LON).done();
        assert!(find_vertex_run(&body, &Near::new(LAT, LON), 2).is_none());
    }

    #[test]
    fn finds_runs_at_odd_offsets() {
        // An FSX apron: u8 surface, u16 count, then vertices on an odd offset.
        let body = Bytes::new()
            .zeros(6)
            .u8(0)
            .u16(3)
            .pos2(LAT, LON)
            .pos2(LAT + 0.001, LON)
            .pos2(LAT, LON + 0.001)
            .done();
        let (at, n) = find_vertex_run(&body, &Near::new(LAT, LON), 3).unwrap();
        assert_eq!(at, 9);
        assert_eq!(n, 3);
        assert!(count_matches(&body, at, n));
    }

    #[test]
    fn count_matches_with_a_second_count_in_between() {
        let body = Bytes::new()
            .zeros(6)
            .u16(2) // vertex count
            .u16(1) // triangle count
            .pos2(LAT, LON)
            .pos2(LAT + 0.001, LON)
            .done();
        let (at, n) = find_vertex_run(&body, &Near::new(LAT, LON), 2).unwrap();
        assert_eq!(n, 2);
        assert!(count_matches(&body, at, n));
    }

    #[test]
    fn rejects_far_coordinates() {
        let body = Bytes::new()
            .zeros(6)
            .pos2(0.0, 0.0)
            .pos2(0.5, 0.5)
            .pos2(1.0, 1.0)
            .done();
        assert!(find_vertex_run(&body, &Near::new(LAT, LON), 2).is_none());
    }

    #[test]
    fn near_handles_antimeridian_and_poles() {
        let n = Near::new(-16.9, 179.95);
        assert!(n.accepts(-16.91, -179.95));
        let polar = Near::new(78.2, 15.5);
        assert!(polar.accepts(78.25, 16.4));
    }
}
