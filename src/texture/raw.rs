//! Bounds-checked little-endian reads over a byte slice.
//!
//! Texture files reach us from third-party add-ons, partial downloads and
//! half-extracted archives, so a truncated or hostile file is normal input, not
//! an exceptional one. Every accessor here reports an error instead of indexing
//! past the end, which is what keeps the rest of the module panic-free.

use super::TextureError;

/// `data[off .. off+len]`, or a `Truncated` error naming the container.
pub(crate) fn slice_at<'a>(
    data: &'a [u8],
    off: usize,
    len: usize,
    container: &'static str,
) -> Result<&'a [u8], TextureError> {
    let truncated = || TextureError::Truncated {
        container,
        offset: off,
        need: len,
        len: data.len(),
    };
    let end = off.checked_add(len).ok_or_else(truncated)?;
    data.get(off..end).ok_or_else(truncated)
}

pub(crate) fn u32_at(data: &[u8], off: usize, container: &'static str) -> Result<u32, TextureError> {
    let b = slice_at(data, off, 4, container)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn u64_at(data: &[u8], off: usize, container: &'static str) -> Result<u64, TextureError> {
    let b = slice_at(data, off, 8, container)?;
    Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

/// A `u64` field narrowed to `usize`, which is a real failure mode for the
/// 64-bit offsets in a KTX2 level index on a 32-bit build.
pub(crate) fn as_usize(v: u64, container: &'static str, what: &str) -> Result<usize, TextureError> {
    usize::try_from(v).map_err(|_| TextureError::Malformed {
        container,
        detail: format!("{what} {v} does not fit in memory"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_within_bounds() {
        let data = [1u8, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(u32_at(&data, 0, "T").unwrap(), 1);
        assert_eq!(u64_at(&data, 4, "T").unwrap(), 2);
        assert_eq!(slice_at(&data, 4, 4, "T").unwrap(), &[2, 0, 0, 0]);
    }

    #[test]
    fn past_end_is_an_error_not_a_panic() {
        let data = [0u8; 3];
        assert!(u32_at(&data, 0, "T").is_err());
        assert!(u64_at(&data, 0, "T").is_err());
        assert!(slice_at(&data, 2, 2, "T").is_err());
        assert!(slice_at(&data, 4, 0, "T").is_err());
    }

    #[test]
    fn offset_overflow_is_an_error() {
        let data = [0u8; 8];
        assert!(slice_at(&data, usize::MAX, 8, "T").is_err());
    }

    #[test]
    fn oversized_u64_is_rejected() {
        assert!(as_usize(0, "T", "offset").is_ok());
        if usize::BITS < 64 {
            assert!(as_usize(u64::MAX, "T", "offset").is_err());
        }
    }
}
