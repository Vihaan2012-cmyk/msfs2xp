//! 128-bit identifiers linking placed scenery objects to their models.
//!
//! MSFS stores GUIDs in the Windows mixed-endian layout: the first three groups
//! are little-endian integers, the last eight bytes are stored as written. The
//! same model therefore appears as `00 08 CE F9 38 4F 70 4D ...` in a placement
//! record and as `{f9ce0800-4f38-4d70-...}` in the ModelInfo XML. Keeping the raw
//! bytes as the canonical form means placements and libraries compare directly.

use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Guid(pub [u8; 16]);

impl Guid {
    pub const NIL: Guid = Guid([0; 16]);

    /// The first sixteen bytes of `bytes`, if there are that many.
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        let raw: [u8; 16] = bytes.get(..16)?.try_into().ok()?;
        Some(Guid(raw))
    }

    pub fn is_nil(&self) -> bool {
        self.0 == [0; 16]
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = &self.0;
        let d1 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let d2 = u16::from_le_bytes([b[4], b[5]]);
        let d3 = u16::from_le_bytes([b[6], b[7]]);
        write!(
            f,
            "{{{d1:08x}-{d2:04x}-{d3:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
            b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
        )
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for Guid {
    type Err = String;

    /// Parse the textual form, with or without braces and dashes.
    fn from_str(s: &str) -> Result<Self, String> {
        let hex: Vec<u8> = s.bytes().filter(u8::is_ascii_hexdigit).collect();
        if hex.len() != 32
            || s.bytes()
                .any(|c| !(c.is_ascii_hexdigit() || matches!(c, b'{' | b'}' | b'-')))
        {
            return Err(format!("not a GUID: {s:?}"));
        }
        let mut text = [0u8; 16];
        for (i, pair) in hex.chunks_exact(2).enumerate() {
            let pair = std::str::from_utf8(pair).map_err(|e| e.to_string())?;
            text[i] = u8::from_str_radix(pair, 16).map_err(|e| e.to_string())?;
        }
        // Text order is big-endian per group; swap the first three groups back.
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&[text[3], text[2], text[1], text[0]]);
        b[4..6].copy_from_slice(&[text[5], text[4]]);
        b[6..8].copy_from_slice(&[text[7], text[6]]);
        b[8..16].copy_from_slice(&text[8..16]);
        Ok(Guid(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bytes and text taken from the same model in a real iniBuilds library.
    const BYTES: [u8; 16] = [
        0x00, 0x08, 0xCE, 0xF9, 0x38, 0x4F, 0x70, 0x4D, 0xA8, 0x7A, 0x38, 0xBC, 0x99, 0x89, 0x9E, 0x0B,
    ];
    const TEXT: &str = "{f9ce0800-4f38-4d70-a87a-38bc99899e0b}";

    #[test]
    fn displays_in_windows_order() {
        assert_eq!(Guid(BYTES).to_string(), TEXT);
    }

    #[test]
    fn parses_back_to_the_stored_bytes() {
        assert_eq!(TEXT.parse::<Guid>().unwrap(), Guid(BYTES));
        assert_eq!(
            "F9CE0800-4F38-4D70-A87A-38BC99899E0B".parse::<Guid>().unwrap(),
            Guid(BYTES)
        );
    }

    #[test]
    fn rejects_malformed_text() {
        assert!("{f9ce0800}".parse::<Guid>().is_err());
        assert!("{f9ce0800-4f38-4d70-a87a-38bc99899e0z}".parse::<Guid>().is_err());
    }

    #[test]
    fn from_slice_needs_sixteen_bytes() {
        assert_eq!(Guid::from_slice(&BYTES), Some(Guid(BYTES)));
        assert!(Guid::from_slice(&BYTES[..15]).is_none());
        assert!(Guid::NIL.is_nil());
        assert!(!Guid(BYTES).is_nil());
    }
}
