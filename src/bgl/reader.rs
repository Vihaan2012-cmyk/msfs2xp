//! A bounds-checked little-endian cursor over a byte slice.
//!
//! Every read returns a `Result`; nothing in this module can panic on malformed
//! input, which is the whole point: third-party BGL files are frequently
//! truncated, padded or built by tools that predate the record layouts we know.

/// Errors produced while reading a BGL byte stream.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum BglError {
    #[error("read past end of data at offset {pos} (needed {need} bytes, have {len})")]
    Eof { pos: usize, need: usize, len: usize },
    #[error("not a BGL file (bad magic number)")]
    BadMagic,
    #[error("malformed BGL: {0}")]
    Malformed(String),
}

/// Little-endian reader over a borrowed slice.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    /// Reader over `data` positioned at `pos` (clamped to the end).
    pub fn at(data: &'a [u8], pos: usize) -> Self {
        Reader {
            data,
            pos: pos.min(data.len()),
        }
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    pub fn seek(&mut self, pos: usize) -> Result<(), BglError> {
        if pos > self.data.len() {
            return Err(BglError::Eof {
                pos,
                need: 0,
                len: self.data.len(),
            });
        }
        self.pos = pos;
        Ok(())
    }

    pub fn skip(&mut self, n: usize) -> Result<(), BglError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(BglError::Malformed("offset overflow".into()))?;
        if end > self.data.len() {
            return Err(BglError::Eof {
                pos: self.pos,
                need: n,
                len: self.data.len(),
            });
        }
        self.pos = end;
        Ok(())
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], BglError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(BglError::Malformed("offset overflow".into()))?;
        if end > self.data.len() {
            return Err(BglError::Eof {
                pos: self.pos,
                need: n,
                len: self.data.len(),
            });
        }
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    /// Peek `n` bytes without advancing.
    pub fn peek(&self, n: usize) -> Result<&'a [u8], BglError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(BglError::Malformed("offset overflow".into()))?;
        if end > self.data.len() {
            return Err(BglError::Eof {
                pos: self.pos,
                need: n,
                len: self.data.len(),
            });
        }
        Ok(&self.data[self.pos..end])
    }

    pub fn u8(&mut self) -> Result<u8, BglError> {
        Ok(self.bytes(1)?[0])
    }

    pub fn i8(&mut self) -> Result<i8, BglError> {
        Ok(self.u8()? as i8)
    }

    pub fn u16(&mut self) -> Result<u16, BglError> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn i16(&mut self) -> Result<i16, BglError> {
        Ok(self.u16()? as i16)
    }

    pub fn u32(&mut self) -> Result<u32, BglError> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Result<i32, BglError> {
        Ok(self.u32()? as i32)
    }

    pub fn u64(&mut self) -> Result<u64, BglError> {
        let b = self.bytes(8)?;
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    pub fn f32(&mut self) -> Result<f32, BglError> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub fn f64(&mut self) -> Result<f64, BglError> {
        Ok(f64::from_bits(self.u64()?))
    }

    /// Read a fixed-size field as a string: NUL-terminated, trailing whitespace
    /// trimmed, invalid UTF-8 replaced (MSFS uses UTF-8, FSX used Latin-1 —
    /// lossy decoding covers the ASCII core both share).
    pub fn string_fixed(&mut self, n: usize) -> Result<String, BglError> {
        let raw = self.bytes(n)?;
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        Ok(decode_str(&raw[..end]))
    }

    /// A sub-reader over `data[start .. start+len]`, positioned at its start.
    pub fn sub(&self, start: usize, len: usize) -> Result<Reader<'a>, BglError> {
        let end = start
            .checked_add(len)
            .ok_or(BglError::Malformed("offset overflow".into()))?;
        if end > self.data.len() {
            return Err(BglError::Eof {
                pos: start,
                need: len,
                len: self.data.len(),
            });
        }
        Ok(Reader::new(&self.data[start..end]))
    }
}

/// Decode a byte string that may be UTF-8 (MSFS) or Latin-1 (FSX/P3D).
pub fn decode_str(raw: &[u8]) -> String {
    let s = match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => raw.iter().map(|&b| b as char).collect(),
    };
    s.trim_end_matches(|c: char| c.is_whitespace() || c == '\0').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_le_values() {
        let data = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06];
        let mut r = Reader::new(&data);
        assert_eq!(r.u8().unwrap(), 0x01);
        assert_eq!(r.u16().unwrap(), 0x0302);
        assert_eq!(r.u8().unwrap(), 0x04);
        assert_eq!(r.u16().unwrap(), 0x0605);
        assert!(r.eof());
    }

    #[test]
    fn reads_floats() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&1234.5f32.to_le_bytes());
        let mut r = Reader::new(&buf);
        assert_eq!(r.f32().unwrap(), 1234.5);
    }

    #[test]
    fn eof_is_error_not_panic() {
        let data = [0u8; 3];
        let mut r = Reader::new(&data);
        assert!(r.u16().is_ok());
        let err = r.u32().unwrap_err();
        assert!(matches!(err, BglError::Eof { .. }));
        // Reader stays usable and does not advance past the end.
        assert_eq!(r.pos(), 2);
        assert_eq!(r.remaining(), 1);
    }

    #[test]
    fn skip_past_end_is_error() {
        let data = [0u8; 4];
        let mut r = Reader::new(&data);
        assert!(r.skip(5).is_err());
        assert_eq!(r.pos(), 0);
        assert!(r.skip(4).is_ok());
    }

    #[test]
    fn sub_window_is_bounded() {
        let data: Vec<u8> = (0u8..16).collect();
        let r = Reader::new(&data);
        let mut w = r.sub(4, 4).unwrap();
        assert_eq!(w.len(), 4);
        assert_eq!(w.u8().unwrap(), 4);
        assert!(r.sub(14, 4).is_err());
    }

    #[test]
    fn string_fixed_trims_nul() {
        let mut data = b"KJFK".to_vec();
        data.extend_from_slice(&[0, 0, 0, 0]);
        let mut r = Reader::new(&data);
        assert_eq!(r.string_fixed(8).unwrap(), "KJFK");
        assert!(r.eof());
    }

    #[test]
    fn string_fixed_handles_invalid_utf8() {
        let data = [b'A', 0xE9, b'B', 0];
        let mut r = Reader::new(&data);
        assert_eq!(r.string_fixed(4).unwrap(), "AéB");
    }

    #[test]
    fn peek_does_not_advance() {
        let data = [1u8, 2, 3];
        let r = Reader::new(&data);
        assert_eq!(r.peek(2).unwrap(), &[1, 2]);
        assert_eq!(r.pos(), 0);
    }
}
