//! Layout probing.
//!
//! Several records carry a fixed-size head followed by a chain of sub-records.
//! The head grew with every simulator generation, and MSFS 2024 grew it again in
//! ways nobody has documented. Rather than hard-coding one length per variant we
//! test each candidate length and keep the one whose sub-record chain parses
//! cleanly all the way to the end of the record. A wrong offset produces a bogus
//! id or a size that overshoots almost immediately, so this is very reliable.

use crate::bgl::file::SubRecords;

/// How well a candidate offset explains the rest of the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeResult {
    pub start: usize,
    /// Sub-records parsed from `start` before the chain broke or ended.
    pub count: usize,
    /// True when the chain ended exactly at the record end.
    pub exact: bool,
    /// True when every id in the chain was a known one.
    pub clean: bool,
}

fn score(data: &[u8], start: usize, valid: fn(u16) -> bool) -> Option<ProbeResult> {
    if start > data.len() {
        return None;
    }
    if start == data.len() {
        // A record with no sub-records at all: valid, but only as a last resort.
        return Some(ProbeResult {
            start,
            count: 0,
            exact: true,
            clean: true,
        });
    }
    let mut it = SubRecords::new(data, start);
    let mut count = 0usize;
    let mut clean = true;
    let mut consumed = start;
    for rec in it.by_ref() {
        if !valid(rec.id) {
            // An unknown id is not fatal: real files contain records nobody has
            // documented. It only makes this candidate less convincing.
            clean = false;
        }
        count += 1;
        consumed = rec.offset + rec.size();
    }
    if count == 0 {
        return None;
    }
    let _ = it;
    Some(ProbeResult {
        start,
        count,
        exact: consumed == data.len(),
        clean,
    })
}

/// Pick the sub-record start offset from a list of candidates.
///
/// A candidate is judged on three things, in order: whether its chain is made
/// only of known record ids, whether it consumes the record exactly, and how
/// many records it yields. A wrong offset normally fails all three at once.
pub fn probe_subrecord_start(data: &[u8], candidates: &[usize], valid: fn(u16) -> bool) -> Option<ProbeResult> {
    let mut best: Option<ProbeResult> = None;
    for &c in candidates {
        if let Some(r) = score(data, c, valid) {
            let better = match best {
                None => true,
                Some(b) => (r.clean, r.exact, r.count) > (b.clean, b.exact, b.count),
            };
            if better {
                best = Some(r);
            }
        }
    }
    best
}

/// Compute the per-element stride for a packed array record.
///
/// `available` is the number of bytes holding `count` elements. If the bytes
/// divide evenly and the result is at least `min_stride`, that is the stride;
/// otherwise fall back to `min_stride` so we at least read the common prefix.
pub fn element_stride(available: usize, count: usize, min_stride: usize) -> (usize, bool) {
    if count == 0 || min_stride == 0 {
        return (min_stride, true);
    }
    let stride = available / count;
    if stride >= min_stride && stride * count == available {
        (stride, true)
    } else {
        (min_stride, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgl::records::ids;
    use crate::bgl::testutil::record;

    fn build(head: usize, subs: &[(u16, usize)]) -> Vec<u8> {
        let mut v = vec![0xAAu8; head];
        for &(id, body) in subs {
            v.extend_from_slice(&record(id, &vec![0u8; body]));
        }
        v
    }

    #[test]
    fn finds_exact_offset() {
        let data = build(62, &[(ids::AP_NAME, 8), (ids::AP_RUNWAY_MSFS, 20)]);
        let r = probe_subrecord_start(&data, &[46, 50, 58, 62], ids::is_airport_subrecord).unwrap();
        assert_eq!(r.start, 62);
        assert_eq!(r.count, 2);
        assert!(r.exact);
    }

    #[test]
    fn prefers_exact_over_partial() {
        let data = build(50, &[(ids::AP_COM, 60)]);
        let r = probe_subrecord_start(&data, &[46, 50, 58, 62], ids::is_airport_subrecord).unwrap();
        assert_eq!(r.start, 50);
        assert!(r.exact);
    }

    #[test]
    fn none_when_nothing_parses() {
        let data = vec![0xFFu8; 80];
        assert!(probe_subrecord_start(&data, &[46, 50], ids::is_airport_subrecord).is_none());
    }

    #[test]
    fn tolerates_an_unknown_record_in_the_chain() {
        let data = build(50, &[(ids::AP_NAME, 4), (0x7777, 4)]);
        let r = probe_subrecord_start(&data, &[46, 50, 62], ids::is_airport_subrecord).unwrap();
        assert_eq!(r.start, 50);
        assert_eq!(r.count, 2);
        assert!(r.exact);
        assert!(!r.clean);
    }

    #[test]
    fn prefers_the_clean_chain_over_a_dirty_one() {
        // Both offsets parse, but only 50 yields known ids.
        let data = build(50, &[(ids::AP_NAME, 8), (ids::AP_COM, 8)]);
        let r = probe_subrecord_start(&data, &[48, 50], ids::is_airport_subrecord).unwrap();
        assert_eq!(r.start, 50);
        assert!(r.clean);
    }

    #[test]
    fn stride_divides_evenly() {
        assert_eq!(element_stride(240, 5, 20), (48, true));
        assert_eq!(element_stride(100, 5, 20), (20, true));
    }

    #[test]
    fn stride_falls_back_when_ragged() {
        assert_eq!(element_stride(103, 5, 20), (20, false));
        assert_eq!(element_stride(30, 5, 20), (20, false));
    }
}
