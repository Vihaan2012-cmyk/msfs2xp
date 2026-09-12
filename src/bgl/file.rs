//! BGL container: header, section table, subsection tables and record slices.
//!
//! Layout (little endian, unchanged from FS2004 through MSFS 2024):
//!
//! ```text
//! 0x00  u32 magic1 = 0x19920201
//! 0x04  u32 header size (0x38)
//! 0x08  u32 filetime low       0x0C  u32 filetime high
//! 0x10  u32 magic2 = 0x08051803
//! 0x14  u32 section count
//! 0x18  32 bytes of QMID bounds
//! 0x38  section headers, 0x14 bytes each:
//!         u32 type, u32 size flag, u32 subsection count, u32 subsection offset, u32 subsection total size
//!       subsection entries are 0x10 or 0x14 bytes (from the size flag):
//!         u32 id, u32 record count, u32 data offset, u32 data size [, u32 unused]
//! ```

use super::reader::{BglError, Reader};
use super::records::ids::{REC_AIRPORT, REC_AIRPORT_MSFS2024};

pub const MAGIC1: u32 = 0x1992_0201;
pub const MAGIC2: u32 = 0x0805_1803;
pub const HEADER_SIZE: usize = 0x38;

pub const SECTION_AIRPORT: u32 = 0x03;
pub const SECTION_AIRPORT_ALT: u32 = 0x3C;
pub const SECTION_SCENERY_OBJECT: u32 = 0x25;
pub const SECTION_NAME_LIST: u32 = 0x27;

/// What a candidate file actually is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileClass {
    /// A readable BGL.
    Bgl,
    /// Magic present but the file is shorter than its own tables claim.
    Truncated,
    /// Not a BGL at all (wrong magic, low entropy).
    NotBgl,
    /// Not a BGL, and the content looks encrypted or compressed (marketplace DRM).
    Encrypted,
}

/// One subsection entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subsection {
    pub id: u32,
    pub record_count: u32,
    pub offset: u32,
    pub size: u32,
}

/// One section of the file.
#[derive(Debug, Clone)]
pub struct Section {
    pub kind: u32,
    pub subsections: Vec<Subsection>,
}

/// A parsed BGL container. Records are returned as borrowed slices.
#[derive(Debug, Clone)]
pub struct BglFile<'a> {
    pub data: &'a [u8],
    pub sections: Vec<Section>,
    pub created: Option<u64>,
}

/// A single top-level record: its id, its offset in the file and the full bytes
/// including the six-byte `u16 id / u32 size` header.
#[derive(Debug, Clone, Copy)]
pub struct RecordSlice<'a> {
    pub id: u16,
    pub offset: usize,
    pub data: &'a [u8],
}

impl<'a> RecordSlice<'a> {
    /// A reader over the record body, positioned just after the 6-byte header.
    pub fn body(&self) -> Reader<'a> {
        Reader::at(self.data, 6)
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }
}

/// Shannon entropy in bits per byte over the sample.
fn entropy(sample: &[u8]) -> f64 {
    if sample.is_empty() {
        return 0.0;
    }
    let mut hist = [0usize; 256];
    for &b in sample {
        hist[b as usize] += 1;
    }
    let n = sample.len() as f64;
    hist.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

/// Classify a candidate file without fully parsing it.
pub fn classify(data: &[u8]) -> FileClass {
    if data.len() < HEADER_SIZE {
        return if data.len() >= 4 && u32::from_le_bytes([data[0], data[1], data[2], data[3]]) == MAGIC1 {
            FileClass::Truncated
        } else {
            FileClass::NotBgl
        };
    }
    let magic1 = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let magic2 = u32::from_le_bytes([data[16], data[17], data[18], data[19]]);
    if magic1 == MAGIC1 && magic2 == MAGIC2 {
        return FileClass::Bgl;
    }
    let sample = &data[..data.len().min(4096)];
    if entropy(sample) > 7.5 {
        FileClass::Encrypted
    } else {
        FileClass::NotBgl
    }
}

impl<'a> BglFile<'a> {
    /// Parse the container structure. Individual malformed sections are dropped
    /// with the rest of the file still usable.
    pub fn parse(data: &'a [u8]) -> Result<Self, BglError> {
        match classify(data) {
            FileClass::Bgl => {}
            FileClass::Truncated => return Err(BglError::Malformed("file truncated".into())),
            _ => return Err(BglError::BadMagic),
        }
        let mut r = Reader::new(data);
        r.skip(4)?; // magic1
        let _header_size = r.u32()?;
        let low = r.u32()? as u64;
        let high = r.u32()? as u64;
        r.skip(4)?; // magic2
        let section_count = r.u32()?;
        r.seek(HEADER_SIZE)?;

        let created = filetime_to_unix(low | (high << 32));
        let mut sections = Vec::new();
        // Guard against absurd counts from corrupt headers.
        let max_sections = ((data.len().saturating_sub(HEADER_SIZE)) / 0x14) as u32;
        for _ in 0..section_count.min(max_sections) {
            let kind = r.u32()?;
            let size_flag = r.u32()?;
            let sub_count = r.u32()?;
            let sub_offset = r.u32()?;
            let _total = r.u32()?;
            // Subsection entry size: 16 bytes normally, 20 when bit 0x10000 is set.
            let sub_size = (((size_flag & 0x10000) | 0x40000) >> 0x0E) as usize;
            let sub_size = if sub_size == 16 || sub_size == 20 { sub_size } else { 16 };
            let mut subs = Vec::new();
            let mut sr = Reader::at(data, sub_offset as usize);
            for _ in 0..sub_count {
                if sr.remaining() < sub_size {
                    break;
                }
                let id = sr.u32()?;
                let record_count = sr.u32()?;
                let offset = sr.u32()?;
                let size = sr.u32()?;
                if sub_size == 20 {
                    sr.skip(4)?;
                }
                subs.push(Subsection {
                    id,
                    record_count,
                    offset,
                    size,
                });
            }
            sections.push(Section {
                kind,
                subsections: subs,
            });
        }
        Ok(BglFile {
            data,
            sections,
            created,
        })
    }

    /// All top-level records in sections of the given kind.
    pub fn records_of(&self, kind: u32) -> Vec<RecordSlice<'a>> {
        let mut out = Vec::new();
        for section in self.sections.iter().filter(|s| s.kind == kind) {
            for sub in &section.subsections {
                let mut pos = sub.offset as usize;
                let end = (sub.offset as usize)
                    .saturating_add(sub.size as usize)
                    .min(self.data.len());
                let mut count = 0u32;
                // `record_count` is advisory; stop on either limit.
                while pos + 6 <= end && (sub.record_count == 0 || count < sub.record_count) {
                    let id = u16::from_le_bytes([self.data[pos], self.data[pos + 1]]);
                    let size = u32::from_le_bytes([
                        self.data[pos + 2],
                        self.data[pos + 3],
                        self.data[pos + 4],
                        self.data[pos + 5],
                    ]) as usize;
                    if size < 6 || pos + size > self.data.len() {
                        break;
                    }
                    out.push(RecordSlice {
                        id,
                        offset: pos,
                        data: &self.data[pos..pos + size],
                    });
                    pos += size;
                    count += 1;
                }
            }
        }
        out
    }

    /// Airport records from section 0x03 (and the rarely used 0x3C alias).
    ///
    /// MSFS 2024 also keeps other top-level records in the airport section
    /// (0x005A carries ground polygons), so only the two airport ids are kept.
    pub fn airport_records(&self) -> Vec<RecordSlice<'a>> {
        let mut out = self.records_of(SECTION_AIRPORT);
        out.extend(self.records_of(SECTION_AIRPORT_ALT));
        out.retain(|r| r.id == REC_AIRPORT || r.id == REC_AIRPORT_MSFS2024);
        out
    }

    pub fn has_section(&self, kind: u32) -> bool {
        self.sections.iter().any(|s| s.kind == kind)
    }
}

fn filetime_to_unix(ft: u64) -> Option<u64> {
    const EPOCH_DIFF: u64 = 11_644_473_600;
    const SECOND: u64 = 10_000_000;
    let secs = ft / SECOND;
    secs.checked_sub(EPOCH_DIFF)
}

/// Iterate the sub-records inside a record body: `(id, full record bytes)`.
///
/// Stops cleanly on a zero or out-of-range size instead of looping forever.
pub struct SubRecords<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> SubRecords<'a> {
    /// `body` must be the full record slice; `start` the offset where sub-records begin.
    pub fn new(data: &'a [u8], start: usize) -> Self {
        SubRecords { data, pos: start }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }
}

impl<'a> Iterator for SubRecords<'a> {
    type Item = RecordSlice<'a>;

    fn next(&mut self) -> Option<RecordSlice<'a>> {
        if self.pos + 6 > self.data.len() {
            return None;
        }
        let id = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        let size = u32::from_le_bytes([
            self.data[self.pos + 2],
            self.data[self.pos + 3],
            self.data[self.pos + 4],
            self.data[self.pos + 5],
        ]) as usize;
        if size < 6 || self.pos + size > self.data.len() {
            return None;
        }
        let rec = RecordSlice {
            id,
            offset: self.pos,
            data: &self.data[self.pos..self.pos + size],
        };
        self.pos += size;
        Some(rec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgl::testutil::{record, BglBuilder};

    #[test]
    fn parses_builder_output() {
        let rec_a = record(0x003C, &[1, 2, 3, 4]);
        let rec_b = record(0x003C, &[5, 6]);
        let data = BglBuilder::new()
            .section(SECTION_AIRPORT, vec![rec_a.clone(), rec_b.clone()])
            .build();
        let file = BglFile::parse(&data).unwrap();
        assert_eq!(file.sections.len(), 1);
        assert_eq!(file.sections[0].kind, SECTION_AIRPORT);
        let recs = file.airport_records();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].id, 0x003C);
        assert_eq!(recs[0].data, &rec_a[..]);
        assert_eq!(recs[1].data, &rec_b[..]);
    }

    #[test]
    fn only_airport_record_ids_are_returned() {
        let data = BglBuilder::new()
            .section(
                SECTION_AIRPORT,
                vec![record(0x0113, &[1]), record(0x005A, &[2, 3]), record(0x003C, &[4])],
            )
            .build();
        let file = BglFile::parse(&data).unwrap();
        let ids: Vec<u16> = file.airport_records().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![0x0113, 0x003C]);
    }

    #[test]
    fn two_sections_are_kept_apart() {
        let data = BglBuilder::new()
            .section(SECTION_AIRPORT, vec![record(0x003C, &[1])])
            .section(SECTION_NAME_LIST, vec![record(0x0027, &[2])])
            .build();
        let file = BglFile::parse(&data).unwrap();
        assert_eq!(file.sections.len(), 2);
        assert_eq!(file.airport_records().len(), 1);
        assert_eq!(file.records_of(SECTION_NAME_LIST).len(), 1);
    }

    #[test]
    fn classify_detects_kinds() {
        let data = BglBuilder::new()
            .section(SECTION_AIRPORT, vec![record(0x3C, &[0])])
            .build();
        assert_eq!(classify(&data), FileClass::Bgl);
        assert_eq!(classify(b"hello world, plain text file here"), FileClass::NotBgl);
        assert_eq!(classify(&[0x01, 0x02, 0x92, 0x19]), FileClass::Truncated);
        // Pseudo-random bytes look encrypted.
        let mut rnd = vec![0u8; 4096];
        let mut x: u32 = 0x1234_5678;
        for b in rnd.iter_mut() {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *b = (x >> 24) as u8;
        }
        assert_eq!(classify(&rnd), FileClass::Encrypted);
    }

    #[test]
    fn truncated_record_is_dropped_not_fatal() {
        let mut data = BglBuilder::new()
            .section(SECTION_AIRPORT, vec![record(0x003C, &[1, 2, 3, 4, 5, 6, 7, 8])])
            .build();
        let len = data.len();
        data.truncate(len - 4);
        let file = BglFile::parse(&data).unwrap();
        assert!(file.airport_records().is_empty());
    }

    #[test]
    fn subrecords_stop_on_zero_size() {
        // id=0x19 size=0 must not loop forever.
        let bytes = [0x19u8, 0x00, 0x00, 0x00, 0x00, 0x00];
        let subs: Vec<_> = SubRecords::new(&bytes, 0).collect();
        assert!(subs.is_empty());
    }

    #[test]
    fn subrecords_iterate() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&record(0x0019, b"NAME"));
        buf.extend_from_slice(&record(0x0004, &[1, 2, 3]));
        let subs: Vec<_> = SubRecords::new(&buf, 0).collect();
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].id, 0x0019);
        assert_eq!(subs[1].id, 0x0004);
        assert_eq!(subs[1].size(), 9);
    }
}
