//! Texture conversion: MSFS KTX2 and DDS in, textures X-Plane 12 loads out.
//!
//! The KTX2 files in MSFS 2024 packages are plain block-compressed data in a
//! KTX2 wrapper (supercompression 0), so no Basis Universal transcoder is
//! involved. Two output paths:
//!
//! - BC1/BC2/BC3 (DXT1/3/5) are passed through as DDS, keeping GPU compression
//!   and the mip chain. X-Plane reads DDS bottom row first, the opposite of the
//!   DirectX convention MSFS uses, so the data is flipped vertically. For these
//!   formats that is a lossless reordering: block rows swap, and the four pixel
//!   rows inside each block swap.
//! - Every other format (BC4/5/6H/7, RGBA) is decoded and written as PNG, whose
//!   row order is unambiguous. X-Plane's BC7 DDS support is not confirmed, so it
//!   is not relied on.
//!
//! Either way the image ends up the way X-Plane expects, so object UVs use one
//! rule for every texture (T = 1 - V).

pub(crate) mod raw;

use raw::{as_usize, slice_at, u32_at, u64_at};

#[derive(thiserror::Error, Debug)]
pub enum TextureError {
    #[error("{container} is truncated: needed {need} bytes at offset {offset}, file has {len}")]
    Truncated {
        container: &'static str,
        offset: usize,
        need: usize,
        len: usize,
    },
    #[error("{container} is malformed: {detail}")]
    Malformed { container: &'static str, detail: String },
    #[error("unsupported texture: {0}")]
    Unsupported(String),
    #[error("PNG encoding failed: {0}")]
    Png(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Ktx2,
    Dds,
    Png,
    Unknown,
}

const KTX2_ID: [u8; 12] = [0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A];

pub fn detect(data: &[u8]) -> SourceFormat {
    if data.starts_with(&KTX2_ID) {
        SourceFormat::Ktx2
    } else if data.starts_with(b"DDS ") {
        SourceFormat::Dds
    } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        SourceFormat::Png
    } else {
        SourceFormat::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Bc1,
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Bc6h,
    Bc7,
    Rgba8,
}

impl PixelFormat {
    /// Bytes per 4x4 block, or `None` for uncompressed data.
    pub fn block_bytes(self) -> Option<usize> {
        match self {
            PixelFormat::Bc1 | PixelFormat::Bc4 => Some(8),
            PixelFormat::Rgba8 => None,
            _ => Some(16),
        }
    }
}

/// Raw texture levels, largest first, in the source's row order (top first).
#[derive(Debug, Clone)]
pub struct TextureImage {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub mips: Vec<Vec<u8>>,
}

fn level_dims(w: u32, h: u32, level: usize) -> (u32, u32) {
    ((w >> level).max(1), (h >> level).max(1))
}

fn level_size(fmt: PixelFormat, w: u32, h: u32) -> usize {
    match fmt.block_bytes() {
        Some(bb) => w.div_ceil(4) as usize * h.div_ceil(4) as usize * bb,
        None => w as usize * h as usize * 4,
    }
}

fn vk_format(vk: u32) -> Option<PixelFormat> {
    Some(match vk {
        131..=134 => PixelFormat::Bc1,
        135 | 136 => PixelFormat::Bc2,
        137 | 138 => PixelFormat::Bc3,
        139 | 140 => PixelFormat::Bc4,
        141 | 142 => PixelFormat::Bc5,
        143 | 144 => PixelFormat::Bc6h,
        145 | 146 => PixelFormat::Bc7,
        37 | 43 => PixelFormat::Rgba8,
        _ => return None,
    })
}

/// Parse a KTX2 container.
pub fn load_ktx2(data: &[u8]) -> Result<TextureImage, TextureError> {
    const C: &str = "KTX2";
    if !data.starts_with(&KTX2_ID) {
        return Err(TextureError::Malformed {
            container: C,
            detail: "bad identifier".into(),
        });
    }
    let vk = u32_at(data, 12, C)?;
    let width = u32_at(data, 20, C)?;
    let height = u32_at(data, 24, C)?.max(1);
    let depth = u32_at(data, 28, C)?;
    let layers = u32_at(data, 32, C)?;
    let faces = u32_at(data, 36, C)?;
    let levels = u32_at(data, 40, C)?.max(1) as usize;
    let scheme = u32_at(data, 44, C)?;
    if scheme != 0 {
        let name = match scheme {
            1 => "BasisLZ",
            2 => "Zstandard",
            3 => "ZLIB",
            _ => "unknown",
        };
        return Err(TextureError::Unsupported(format!(
            "KTX2 supercompression {scheme} ({name})"
        )));
    }
    if depth > 1 || layers > 1 || faces != 1 {
        return Err(TextureError::Unsupported("KTX2 arrays, cubes and volumes".into()));
    }
    if width == 0 || levels > 16 {
        return Err(TextureError::Malformed {
            container: C,
            detail: format!("{width}x{height} with {levels} levels"),
        });
    }
    let format = vk_format(vk).ok_or_else(|| TextureError::Unsupported(format!("KTX2 vkFormat {vk}")))?;
    let mut mips = Vec::with_capacity(levels);
    for i in 0..levels {
        let entry = 80 + i * 24;
        let offset = as_usize(u64_at(data, entry, C)?, C, "level offset")?;
        let length = as_usize(u64_at(data, entry + 8, C)?, C, "level length")?;
        let (w, h) = level_dims(width, height, i);
        let expected = level_size(format, w, h);
        if length < expected {
            return Err(TextureError::Malformed {
                container: C,
                detail: format!("level {i} holds {length} bytes, needs {expected}"),
            });
        }
        mips.push(slice_at(data, offset, expected, C)?.to_vec());
    }
    Ok(TextureImage {
        width,
        height,
        format,
        mips,
    })
}

/// Parse a DDS file (classic header, optionally with the DX10 extension).
pub fn load_dds(data: &[u8]) -> Result<TextureImage, TextureError> {
    const C: &str = "DDS";
    if !data.starts_with(b"DDS ") || u32_at(data, 4, C)? != 124 {
        return Err(TextureError::Malformed {
            container: C,
            detail: "bad header".into(),
        });
    }
    let height = u32_at(data, 12, C)?.max(1);
    let width = u32_at(data, 16, C)?.max(1);
    let mip_count = (u32_at(data, 28, C)?.max(1) as usize).min(16);
    let pf_flags = u32_at(data, 80, C)?;
    let four_cc = slice_at(data, 84, 4, C)?;
    let mut start = 128;
    let (format, bgra) = if pf_flags & 0x4 != 0 {
        match four_cc {
            b"DXT1" => (PixelFormat::Bc1, false),
            b"DXT2" | b"DXT3" => (PixelFormat::Bc2, false),
            b"DXT4" | b"DXT5" => (PixelFormat::Bc3, false),
            b"ATI1" | b"BC4U" => (PixelFormat::Bc4, false),
            b"ATI2" | b"BC5U" => (PixelFormat::Bc5, false),
            b"DX10" => {
                start = 148;
                let dxgi = u32_at(data, 128, C)?;
                match dxgi {
                    70..=72 => (PixelFormat::Bc1, false),
                    73..=75 => (PixelFormat::Bc2, false),
                    76..=78 => (PixelFormat::Bc3, false),
                    79..=81 => (PixelFormat::Bc4, false),
                    82..=84 => (PixelFormat::Bc5, false),
                    94..=96 => (PixelFormat::Bc6h, false),
                    97..=99 => (PixelFormat::Bc7, false),
                    27..=29 => (PixelFormat::Rgba8, false),
                    87 | 91 => (PixelFormat::Rgba8, true),
                    other => return Err(TextureError::Unsupported(format!("DDS DXGI format {other}"))),
                }
            }
            other => {
                return Err(TextureError::Unsupported(format!(
                    "DDS FourCC {:?}",
                    String::from_utf8_lossy(other)
                )))
            }
        }
    } else if u32_at(data, 88, C)? == 32 {
        // Uncompressed 32-bit: the red mask says whether it is RGBA or BGRA.
        (PixelFormat::Rgba8, u32_at(data, 92, C)? == 0x00FF_0000)
    } else {
        return Err(TextureError::Unsupported("DDS pixel format".into()));
    };

    let mut mips = Vec::with_capacity(mip_count);
    let mut pos = start;
    for i in 0..mip_count {
        let (w, h) = level_dims(width, height, i);
        let n = level_size(format, w, h);
        let Ok(level) = slice_at(data, pos, n, C) else {
            if i == 0 {
                return Err(TextureError::Truncated {
                    container: C,
                    offset: pos,
                    need: n,
                    len: data.len(),
                });
            }
            break; // a short mip chain is still a usable texture
        };
        let mut level = level.to_vec();
        if bgra {
            for px in level.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        mips.push(level);
        pos += n;
    }
    Ok(TextureImage {
        width,
        height,
        format,
        mips,
    })
}

/// Parse either container.
pub fn load(data: &[u8]) -> Result<TextureImage, TextureError> {
    match detect(data) {
        SourceFormat::Ktx2 => load_ktx2(data),
        SourceFormat::Dds => load_dds(data),
        SourceFormat::Png => Err(TextureError::Unsupported("PNG needs no conversion".into())),
        SourceFormat::Unknown => Err(TextureError::Unsupported("not a KTX2, DDS or PNG file".into())),
    }
}

/// Decode one level to RGBA8, top row first.
pub fn decode_rgba8(img: &TextureImage, level: usize) -> Result<Vec<u8>, TextureError> {
    let data = img
        .mips
        .get(level)
        .ok_or_else(|| TextureError::Unsupported(format!("no mip level {level}")))?;
    let (w, h) = level_dims(img.width, img.height, level);
    let (w, h) = (w as usize, h as usize);
    let Some(bb) = img.format.block_bytes() else {
        return Ok(data.clone());
    };
    let decode: fn(&[u8], &mut [u32]) = match img.format {
        PixelFormat::Bc1 => texture2ddecoder::decode_bc1_block,
        PixelFormat::Bc2 => texture2ddecoder::decode_bc2_block,
        PixelFormat::Bc3 => texture2ddecoder::decode_bc3_block,
        PixelFormat::Bc4 => texture2ddecoder::decode_bc4_block,
        PixelFormat::Bc5 => texture2ddecoder::decode_bc5_block,
        PixelFormat::Bc6h => texture2ddecoder::decode_bc6_block_unsigned,
        PixelFormat::Bc7 => texture2ddecoder::decode_bc7_block,
        PixelFormat::Rgba8 => unreachable!("handled above"),
    };
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    let mut out = vec![0u8; w * h * 4];
    let mut block = [0u32; 16];
    for by in 0..bh {
        for bx in 0..bw {
            let at = (by * bw + bx) * bb;
            let Some(src) = data.get(at..at + bb) else {
                return Err(TextureError::Truncated {
                    container: "texture level",
                    offset: at,
                    need: bb,
                    len: data.len(),
                });
            };
            decode(src, &mut block);
            for py in 0..4 {
                for px in 0..4 {
                    let (x, y) = (bx * 4 + px, by * 4 + py);
                    if x < w && y < h {
                        // The decoder packs pixels as little-endian B, G, R, A.
                        let [b, g, r, a] = block[py * 4 + px].to_le_bytes();
                        let o = (y * w + x) * 4;
                        out[o..o + 4].copy_from_slice(&[r, g, b, a]);
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Flip the 4x4 pixel rows of a 64-bit BC1-style colour block.
fn flip_colour_block(b: &mut [u8]) {
    b[4..8].reverse(); // one index byte per pixel row
}

/// Flip a BC3 alpha block: 3-bit indices, 12 bits per pixel row.
fn flip_bc3_alpha(b: &mut [u8]) {
    let mut bits = 0u64;
    for (i, &byte) in b[2..8].iter().enumerate() {
        bits |= (byte as u64) << (8 * i);
    }
    let rows: Vec<u64> = (0..4).map(|r| (bits >> (12 * r)) & 0xFFF).collect();
    let mut flipped = 0u64;
    for (r, row) in rows.iter().rev().enumerate() {
        flipped |= row << (12 * r);
    }
    for (i, byte) in b[2..8].iter_mut().enumerate() {
        *byte = (flipped >> (8 * i)) as u8;
    }
}

/// Vertically flip one level of BC1/BC2/BC3 data without decoding it.
///
/// Only possible when the height is a whole number of blocks; otherwise the
/// padding rows of the last block would move into view.
pub fn flip_bc_level(fmt: PixelFormat, data: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    if h % 4 != 0 || !matches!(fmt, PixelFormat::Bc1 | PixelFormat::Bc2 | PixelFormat::Bc3) {
        return None;
    }
    let bb = fmt.block_bytes()?;
    let (bw, bh) = (w.div_ceil(4) as usize, (h / 4) as usize);
    let row_bytes = bw * bb;
    if data.len() < row_bytes * bh {
        return None;
    }
    let mut out = Vec::with_capacity(row_bytes * bh);
    for row in (0..bh).rev() {
        let mut blocks = data[row * row_bytes..(row + 1) * row_bytes].to_vec();
        for b in blocks.chunks_exact_mut(bb) {
            match fmt {
                PixelFormat::Bc1 => flip_colour_block(b),
                PixelFormat::Bc2 => {
                    // Explicit 4-bit alpha, two bytes per pixel row, then colour.
                    let alpha: Vec<[u8; 2]> = b[0..8].chunks_exact(2).map(|c| [c[0], c[1]]).rev().collect();
                    for (i, pair) in alpha.iter().enumerate() {
                        b[i * 2..i * 2 + 2].copy_from_slice(pair);
                    }
                    flip_colour_block(&mut b[8..16]);
                }
                PixelFormat::Bc3 => {
                    flip_bc3_alpha(&mut b[0..8]);
                    flip_colour_block(&mut b[8..16]);
                }
                _ => return None,
            }
        }
        out.extend_from_slice(&blocks);
    }
    Some(out)
}

/// Write BC1/BC2/BC3 data as an X-Plane-oriented DDS, flipping each level.
pub fn to_xplane_dds(img: &TextureImage) -> Result<Vec<u8>, TextureError> {
    let four_cc: &[u8; 4] = match img.format {
        PixelFormat::Bc1 => b"DXT1",
        PixelFormat::Bc2 => b"DXT3",
        PixelFormat::Bc3 => b"DXT5",
        other => {
            return Err(TextureError::Unsupported(format!(
                "{other:?} cannot be written as legacy DDS"
            )))
        }
    };
    let mut levels = Vec::new();
    for (i, data) in img.mips.iter().enumerate() {
        let (w, h) = level_dims(img.width, img.height, i);
        match flip_bc_level(img.format, data, w, h) {
            Some(f) => levels.push(f),
            None => break, // tiny levels under 4 pixels tall are dropped
        }
    }
    if levels.is_empty() {
        return Err(TextureError::Unsupported(format!(
            "{}x{} is not a whole number of blocks",
            img.width, img.height
        )));
    }
    let mut out = Vec::with_capacity(128 + levels.iter().map(Vec::len).sum::<usize>());
    let put = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    out.extend_from_slice(b"DDS ");
    put(&mut out, 124);
    // CAPS | HEIGHT | WIDTH | PIXELFORMAT | MIPMAPCOUNT | LINEARSIZE
    put(&mut out, 0x1 | 0x2 | 0x4 | 0x1000 | 0x20000 | 0x80000);
    put(&mut out, img.height);
    put(&mut out, img.width);
    put(&mut out, levels[0].len() as u32);
    put(&mut out, 0); // depth
    put(&mut out, levels.len() as u32);
    out.extend_from_slice(&[0u8; 44]); // reserved
    put(&mut out, 32); // pixel format size
    put(&mut out, 0x4); // FOURCC
    out.extend_from_slice(four_cc);
    out.extend_from_slice(&[0u8; 20]); // bit count and masks
    let caps = 0x1000 | if levels.len() > 1 { 0x8 | 0x40_0000 } else { 0 };
    put(&mut out, caps);
    out.extend_from_slice(&[0u8; 16]); // caps2-4, reserved
    debug_assert_eq!(out.len(), 128);
    for l in levels {
        out.extend_from_slice(&l);
    }
    Ok(out)
}

/// Encode level 0 as a PNG (top row first, as PNG always is).
pub fn to_png(img: &TextureImage) -> Result<Vec<u8>, TextureError> {
    let rgba = decode_rgba8(img, 0)?;
    let mut out = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    image::ImageEncoder::write_image(encoder, &rgba, img.width, img.height, image::ExtendedColorType::Rgba8)
        .map_err(|e| TextureError::Png(e.to_string()))?;
    Ok(out)
}

/// A converted texture and the file extension to give it.
#[derive(Debug, Clone)]
pub struct Converted {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
}

/// Convert any MSFS texture into something X-Plane 12 loads.
pub fn convert_for_xplane(data: &[u8]) -> Result<Converted, TextureError> {
    if detect(data) == SourceFormat::Png {
        return Ok(Converted {
            bytes: data.to_vec(),
            extension: "png",
        });
    }
    let img = load(data)?;
    if matches!(img.format, PixelFormat::Bc1 | PixelFormat::Bc2 | PixelFormat::Bc3) {
        if let Ok(bytes) = to_xplane_dds(&img) {
            return Ok(Converted {
                bytes,
                extension: "dds",
            });
        }
    }
    Ok(Converted {
        bytes: to_png(&img)?,
        extension: "png",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random bytes.
    fn noise(n: usize, seed: u32) -> Vec<u8> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 24) as u8
            })
            .collect()
    }

    fn ktx2(vk: u32, w: u32, h: u32, levels: &[Vec<u8>], scheme: u32) -> Vec<u8> {
        let mut out = KTX2_ID.to_vec();
        for v in [vk, 1, w, h, 0, 0, 1, levels.len() as u32, scheme] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.resize(80, 0);
        let mut offset = 80 + levels.len() * 24;
        for l in levels {
            out.extend_from_slice(&(offset as u64).to_le_bytes());
            out.extend_from_slice(&(l.len() as u64).to_le_bytes());
            out.extend_from_slice(&(l.len() as u64).to_le_bytes());
            offset += l.len();
        }
        for l in levels {
            out.extend_from_slice(l);
        }
        out
    }

    fn vflip(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
        (0..h)
            .rev()
            .flat_map(|y| rgba[y * w * 4..(y + 1) * w * 4].to_vec())
            .collect()
    }

    #[test]
    fn detects_containers() {
        assert_eq!(detect(&KTX2_ID), SourceFormat::Ktx2);
        assert_eq!(detect(b"DDS xxxx"), SourceFormat::Dds);
        assert_eq!(detect(&[0x89, b'P', b'N', b'G', 1]), SourceFormat::Png);
        assert_eq!(detect(b"nope"), SourceFormat::Unknown);
    }

    #[test]
    fn loads_a_bc7_ktx2_like_msfs_ships() {
        let data = ktx2(145, 8, 8, &[noise(64, 1), noise(16, 2)], 0);
        let img = load(&data).unwrap();
        assert_eq!((img.width, img.height, img.format), (8, 8, PixelFormat::Bc7));
        assert_eq!(img.mips.len(), 2);
        assert_eq!(img.mips[1], noise(16, 2));
    }

    #[test]
    fn rejects_supercompressed_and_unknown_formats() {
        let e = load(&ktx2(145, 4, 4, &[noise(16, 1)], 1)).unwrap_err();
        assert!(e.to_string().contains("BasisLZ"), "{e}");
        let e = load(&ktx2(999, 4, 4, &[noise(16, 1)], 0)).unwrap_err();
        assert!(e.to_string().contains("999"), "{e}");
    }

    #[test]
    fn truncated_files_error_instead_of_panicking() {
        let mut data = ktx2(133, 8, 8, &[noise(32, 1)], 0);
        data.truncate(data.len() - 5);
        assert!(load(&data).is_err());
        assert!(load(&data[..30]).is_err());
        assert!(load_dds(b"DDS \x7c\x00\x00\x00short").is_err());
    }

    #[test]
    fn block_flips_match_a_decoded_flip() {
        // For every format the DDS path uses, flipping the blocks must give the
        // same pixels as decoding and flipping the image.
        for (fmt, bb) in [(PixelFormat::Bc1, 8), (PixelFormat::Bc2, 16), (PixelFormat::Bc3, 16)] {
            let (w, h) = (8u32, 12u32);
            let data = noise(2 * 3 * bb, 7 + bb as u32);
            let img = TextureImage {
                width: w,
                height: h,
                format: fmt,
                mips: vec![data.clone()],
            };
            let flipped = TextureImage {
                mips: vec![flip_bc_level(fmt, &data, w, h).unwrap()],
                ..img.clone()
            };
            let expected = vflip(&decode_rgba8(&img, 0).unwrap(), w as usize, h as usize);
            assert_eq!(decode_rgba8(&flipped, 0).unwrap(), expected, "{fmt:?}");
        }
    }

    #[test]
    fn heights_that_are_not_whole_blocks_cannot_be_flipped() {
        assert!(flip_bc_level(PixelFormat::Bc1, &noise(8, 1), 4, 2).is_none());
        assert!(flip_bc_level(PixelFormat::Bc7, &noise(16, 1), 4, 4).is_none());
    }

    #[test]
    fn bc1_becomes_a_dds_that_reads_back() {
        let data = ktx2(133, 8, 8, &[noise(32, 3), noise(8, 4), noise(8, 5), noise(8, 6)], 0);
        let out = convert_for_xplane(&data).unwrap();
        assert_eq!(out.extension, "dds");
        let back = load_dds(&out.bytes).unwrap();
        assert_eq!((back.width, back.height, back.format), (8, 8, PixelFormat::Bc1));
        // The 2x2 and 1x1 levels are shorter than a block and are dropped.
        assert_eq!(back.mips.len(), 2);
    }

    #[test]
    fn bc7_becomes_a_png_of_the_right_size() {
        let data = ktx2(145, 8, 4, &[noise(32, 9)], 0);
        let out = convert_for_xplane(&data).unwrap();
        assert_eq!(out.extension, "png");
        let decoded = image::load_from_memory(&out.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (8, 4));
    }

    #[test]
    fn png_passes_through() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(convert_for_xplane(&png).unwrap().bytes, png);
    }

    /// Convert every texture in `MSFS2XP_TEXTURES` and report the split.
    #[test]
    #[ignore]
    fn real_textures_convert() {
        let Ok(dir) = std::env::var("MSFS2XP_TEXTURES") else {
            return;
        };
        let (mut dds, mut png, mut failed) = (0, 0, 0);
        for entry in std::fs::read_dir(&dir).unwrap().filter_map(Result::ok) {
            let p = entry.path();
            if !p.to_string_lossy().to_ascii_lowercase().ends_with(".ktx2") {
                continue;
            }
            match convert_for_xplane(&std::fs::read(&p).unwrap()) {
                Ok(c) if c.extension == "dds" => dds += 1,
                Ok(_) => png += 1,
                Err(e) => {
                    failed += 1;
                    eprintln!("{}: {e}", p.display());
                }
            }
        }
        eprintln!("dds {dds}, png {png}, failed {failed}");
        assert_eq!(failed, 0);
    }
}
