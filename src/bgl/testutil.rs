//! Helpers to build synthetic BGL byte streams for tests.

use super::file::{HEADER_SIZE, MAGIC1, MAGIC2};

/// Build one record: `u16 id`, `u32 total size`, then the body.
pub fn record(id: u16, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + body.len());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&((body.len() + 6) as u32).to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// Accumulates sections of records and emits a valid container around them.
#[derive(Default)]
pub struct BglBuilder {
    sections: Vec<(u32, Vec<Vec<u8>>)>,
}

impl BglBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn section(mut self, kind: u32, records: Vec<Vec<u8>>) -> Self {
        self.sections.push((kind, records));
        self
    }

    pub fn build(self) -> Vec<u8> {
        let n = self.sections.len();
        let sub_table_start = HEADER_SIZE + n * 0x14;
        let data_start = sub_table_start + n * 0x10;

        let mut data = Vec::new();
        let mut subs = Vec::new();
        for (_, records) in &self.sections {
            let offset = data_start + data.len();
            let mut size = 0usize;
            for rec in records {
                data.extend_from_slice(rec);
                size += rec.len();
            }
            subs.push((records.len() as u32, offset as u32, size as u32));
        }

        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC1.to_le_bytes());
        out.extend_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&MAGIC2.to_le_bytes());
        out.extend_from_slice(&(n as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 32]);
        debug_assert_eq!(out.len(), HEADER_SIZE);

        for (i, (kind, _)) in self.sections.iter().enumerate() {
            let sub_off = sub_table_start + i * 0x10;
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes()); // size flag -> 16-byte subsections
            out.extend_from_slice(&1u32.to_le_bytes()); // one subsection per section
            out.extend_from_slice(&(sub_off as u32).to_le_bytes());
            out.extend_from_slice(&0x10u32.to_le_bytes());
        }
        for (i, (count, offset, size)) in subs.iter().enumerate() {
            out.extend_from_slice(&(i as u32).to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
        }
        debug_assert_eq!(out.len(), data_start);
        out.extend_from_slice(&data);
        out
    }
}

/// Little-endian byte writer used to compose record bodies in tests.
#[derive(Default, Clone)]
pub struct Bytes(pub Vec<u8>);

impl Bytes {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    pub fn u16(mut self, v: u16) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u32(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn i32(mut self, v: i32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn f32(mut self, v: f32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn zeros(mut self, n: usize) -> Self {
        self.0.extend(std::iter::repeat_n(0u8, n));
        self
    }
    pub fn raw(mut self, b: &[u8]) -> Self {
        self.0.extend_from_slice(b);
        self
    }
    /// A packed lon/lat/alt triple as records store it.
    pub fn pos(self, lat: f64, lon: f64, alt_m: f64) -> Self {
        use crate::bgl::codec::{lat_to_u32, lon_to_u32};
        self.u32(lon_to_u32(lon))
            .u32(lat_to_u32(lat))
            .i32((alt_m * 1000.0).round() as i32)
    }
    /// A packed lon/lat pair without altitude.
    pub fn pos2(self, lat: f64, lon: f64) -> Self {
        use crate::bgl::codec::{lat_to_u32, lon_to_u32};
        self.u32(lon_to_u32(lon)).u32(lat_to_u32(lat))
    }
    pub fn done(self) -> Vec<u8> {
        self.0
    }
}
