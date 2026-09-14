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
    /// BC4 holding signed values (-1..1).
    Bc4s,
    Bc5,
    /// BC5 holding signed values, as MSFS normal maps do.
    Bc5s,
    Bc6h,
    Bc7,
    Rgba8,
}

impl PixelFormat {
    /// Bytes per 4x4 block, or `None` for uncompressed data.
    pub fn block_bytes(self) -> Option<usize> {
        match self {
            PixelFormat::Bc1 | PixelFormat::Bc4 | PixelFormat::Bc4s => Some(8),
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
        139 => PixelFormat::Bc4,
        140 => PixelFormat::Bc4s,
        141 => PixelFormat::Bc5,
        142 => PixelFormat::Bc5s,
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
            b"BC4S" => (PixelFormat::Bc4s, false),
            b"ATI2" | b"BC5U" => (PixelFormat::Bc5, false),
            b"BC5S" => (PixelFormat::Bc5s, false),
            b"DX10" => {
                start = 148;
                let dxgi = u32_at(data, 128, C)?;
                match dxgi {
                    70..=72 => (PixelFormat::Bc1, false),
                    73..=75 => (PixelFormat::Bc2, false),
                    76..=78 => (PixelFormat::Bc3, false),
                    79 | 80 => (PixelFormat::Bc4, false),
                    81 => (PixelFormat::Bc4s, false),
                    82 | 83 => (PixelFormat::Bc5, false),
                    84 => (PixelFormat::Bc5s, false),
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
    // One- and two-channel formats are decoded here, signed ones included.
    let channels = match img.format {
        PixelFormat::Bc4 => Some((1, false)),
        PixelFormat::Bc4s => Some((1, true)),
        PixelFormat::Bc5 => Some((2, false)),
        PixelFormat::Bc5s => Some((2, true)),
        _ => None,
    };
    if let Some((n, signed)) = channels {
        let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
        let mut out = vec![0u8; w * h * 4];
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
                let r = bc4_block(&src[0..8], signed);
                let g = if n == 2 { bc4_block(&src[8..16], signed) } else { r };
                for py in 0..4 {
                    for px in 0..4 {
                        let (x, y) = (bx * 4 + px, by * 4 + py);
                        if x < w && y < h {
                            let (i, o) = (py * 4 + px, (y * w + x) * 4);
                            let b = if n == 1 { r[i] } else { 0 };
                            out[o..o + 4].copy_from_slice(&[r[i], g[i], b, 255]);
                        }
                    }
                }
            }
        }
        return Ok(out);
    }
    let decode: fn(&[u8], &mut [u32]) = match img.format {
        PixelFormat::Bc1 => texture2ddecoder::decode_bc1_block,
        PixelFormat::Bc2 => texture2ddecoder::decode_bc2_block,
        PixelFormat::Bc3 => texture2ddecoder::decode_bc3_block,
        PixelFormat::Bc4 | PixelFormat::Bc4s | PixelFormat::Bc5 | PixelFormat::Bc5s => unreachable!("decoded above"),
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

/// Decode one BC4 block (one channel) to 16 values. Signed data is mapped so
/// that -1 is 0 and +1 is 255.
fn bc4_block(b: &[u8], signed: bool) -> [u8; 16] {
    let raw = |x: u8| if signed { x as i8 as i16 } else { x as i16 };
    let unit = |x: u8| {
        if signed {
            (x as i8 as f32 / 127.0).max(-1.0)
        } else {
            x as f32 / 255.0
        }
    };
    let (e0, e1) = (unit(b[0]), unit(b[1]));
    let mut pal = [e0, e1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    if raw(b[0]) > raw(b[1]) {
        for i in 1..7 {
            pal[i + 1] = ((7 - i) as f32 * e0 + i as f32 * e1) / 7.0;
        }
    } else {
        for i in 1..5 {
            pal[i + 1] = ((5 - i) as f32 * e0 + i as f32 * e1) / 5.0;
        }
        pal[6] = if signed { -1.0 } else { 0.0 };
        pal[7] = 1.0;
    }
    let mut bits = 0u64;
    for (i, &x) in b[2..8].iter().enumerate() {
        bits |= (x as u64) << (8 * i);
    }
    let mut out = [0u8; 16];
    for (p, o) in out.iter_mut().enumerate() {
        let v = pal[((bits >> (3 * p)) & 7) as usize];
        let v = if signed { v * 0.5 + 0.5 } else { v };
        *o = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    out
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

/// Mirror a BC1-style colour block left to right: two bits per pixel, one
/// index byte per row.
fn mirror_colour_block(b: &mut [u8]) {
    for byte in &mut b[4..8] {
        let v = *byte;
        let mut o = 0u8;
        for x in 0..4 {
            o |= ((v >> (2 * x)) & 3) << (2 * (3 - x));
        }
        *byte = o;
    }
}

/// Mirror a BC2 explicit alpha block: four bits per pixel, a u16 per row.
fn mirror_bc2_alpha(b: &mut [u8]) {
    for r in 0..4 {
        let v = u16::from_le_bytes([b[2 * r], b[2 * r + 1]]);
        let mut o = 0u16;
        for x in 0..4 {
            o |= ((v >> (4 * x)) & 0xF) << (4 * (3 - x));
        }
        b[2 * r..2 * r + 2].copy_from_slice(&o.to_le_bytes());
    }
}

/// Mirror a BC3 alpha block: three-bit indices, 12 bits per pixel row.
fn mirror_bc3_alpha(b: &mut [u8]) {
    let mut bits = 0u64;
    for (i, &byte) in b[2..8].iter().enumerate() {
        bits |= (byte as u64) << (8 * i);
    }
    let mut out = 0u64;
    for r in 0..4 {
        let row = (bits >> (12 * r)) & 0xFFF;
        let mut m = 0u64;
        for x in 0..4 {
            m |= ((row >> (3 * x)) & 7) << (3 * (3 - x));
        }
        out |= m << (12 * r);
    }
    for (i, byte) in b[2..8].iter_mut().enumerate() {
        *byte = (out >> (8 * i)) as u8;
    }
}

/// Mirror one level of BC1/BC2/BC3 data left to right without decoding it.
/// Only possible when the width is a whole number of blocks.
pub fn mirror_bc_level(fmt: PixelFormat, data: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    if w % 4 != 0 || !matches!(fmt, PixelFormat::Bc1 | PixelFormat::Bc2 | PixelFormat::Bc3) {
        return None;
    }
    let bb = fmt.block_bytes()?;
    let (bw, bh) = ((w / 4) as usize, h.div_ceil(4) as usize);
    let row_bytes = bw * bb;
    if data.len() < row_bytes * bh {
        return None;
    }
    let mut out = Vec::with_capacity(row_bytes * bh);
    for row in 0..bh {
        let blocks = &data[row * row_bytes..(row + 1) * row_bytes];
        for block in blocks.chunks_exact(bb).rev() {
            let mut b = block.to_vec();
            match fmt {
                PixelFormat::Bc1 => mirror_colour_block(&mut b),
                PixelFormat::Bc2 => {
                    mirror_bc2_alpha(&mut b[0..8]);
                    mirror_colour_block(&mut b[8..16]);
                }
                _ => {
                    mirror_bc3_alpha(&mut b[0..8]);
                    mirror_colour_block(&mut b[8..16]);
                }
            }
            out.extend_from_slice(&b);
        }
    }
    Some(out)
}

/// Flip a converted texture file (the DDS or PNG this converter writes)
/// top to bottom and/or left to right. DDS data is rearranged block by
/// block, losslessly; mip levels too small to rearrange are dropped.
pub fn transform_texture_file(data: &[u8], flip_v: bool, flip_h: bool) -> Result<Vec<u8>, TextureError> {
    if !flip_v && !flip_h {
        return Ok(data.to_vec());
    }
    match detect(data) {
        SourceFormat::Png => {
            let mut img = image::load_from_memory_with_format(data, image::ImageFormat::Png)
                .map_err(|e| TextureError::Png(e.to_string()))?
                .to_rgba8();
            if flip_v {
                img = image::imageops::flip_vertical(&img);
            }
            if flip_h {
                img = image::imageops::flip_horizontal(&img);
            }
            let (w, h) = img.dimensions();
            let mut out = Vec::new();
            let encoder = image::codecs::png::PngEncoder::new(&mut out);
            image::ImageEncoder::write_image(encoder, img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
                .map_err(|e| TextureError::Png(e.to_string()))?;
            Ok(out)
        }
        SourceFormat::Dds => {
            let img = load_dds(data)?;
            let mut levels = Vec::new();
            for (i, level) in img.mips.iter().enumerate() {
                let (w, h) = level_dims(img.width, img.height, i);
                let mut l = Some(level.clone());
                if flip_v {
                    l = l.and_then(|d| flip_bc_level(img.format, &d, w, h));
                }
                if flip_h {
                    l = l.and_then(|d| mirror_bc_level(img.format, &d, w, h));
                }
                match l {
                    Some(d) => levels.push(d),
                    None if i == 0 => {
                        return Err(TextureError::Unsupported(format!(
                            "{}x{} {:?} cannot be flipped block by block",
                            img.width, img.height, img.format
                        )))
                    }
                    None => break,
                }
            }
            // Same header, with the mip count of what is left.
            let mut out = data[..128].to_vec();
            out[28..32].copy_from_slice(&(levels.len() as u32).to_le_bytes());
            for l in levels {
                out.extend_from_slice(&l);
            }
            Ok(out)
        }
        _ => Err(TextureError::Unsupported("only converted DDS and PNG textures can be flipped".into())),
    }
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
    /// Roughly what the texture occupies in video memory, mip chain included.
    pub vram_bytes: usize,
}

/// Convert any MSFS texture into something X-Plane 12 loads.
pub fn convert_for_xplane(data: &[u8]) -> Result<Converted, TextureError> {
    convert_for_xplane_capped(data, u32::MAX)
}

/// Whether a texture goes out as DDS: BC1-3 whose top level flips cleanly.
fn writes_dds(img: &TextureImage) -> bool {
    matches!(img.format, PixelFormat::Bc1 | PixelFormat::Bc2 | PixelFormat::Bc3)
        && img
            .mips
            .first()
            .is_some_and(|m| flip_bc_level(img.format, m, img.width, img.height).is_some())
}

/// The extension [`convert_for_xplane_capped`] gives this texture at any cap,
/// so objects can name a texture before it is written.
pub fn output_extension(data: &[u8]) -> Result<&'static str, TextureError> {
    if detect(data) == SourceFormat::Png {
        return Ok("png");
    }
    Ok(if writes_dds(&load(data)?) { "dds" } else { "png" })
}

/// The first level whose longer side fits `max_side`, or the smallest level
/// allowed. DDS output needs every level a whole number of blocks tall.
fn first_level_within(img: &TextureImage, max_side: u32, whole_blocks: bool) -> usize {
    let mut best = 0;
    for i in 0..img.mips.len() {
        let (w, h) = level_dims(img.width, img.height, i);
        if i > 0 && whole_blocks && h % 4 != 0 {
            break;
        }
        best = i;
        if w.max(h) <= max_side {
            break;
        }
    }
    best
}

/// The image from level `first` down.
fn from_level(img: &TextureImage, first: usize) -> TextureImage {
    let (width, height) = level_dims(img.width, img.height, first);
    TextureImage {
        width,
        height,
        format: img.format,
        mips: img.mips[first..].to_vec(),
    }
}

/// Encode RGBA8 as PNG, shrunk to fit `max_side` when it is larger.
fn png_within(rgba: Vec<u8>, w: u32, h: u32, max_side: u32) -> Result<Converted, TextureError> {
    let mut img = image::RgbaImage::from_raw(w, h, rgba)
        .ok_or_else(|| TextureError::Unsupported("pixel data does not match its size".into()))?;
    if w.max(h) > max_side {
        let k = max_side as f64 / w.max(h) as f64;
        let nw = ((w as f64 * k).round() as u32).max(1);
        let nh = ((h as f64 * k).round() as u32).max(1);
        img = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
    }
    let (w, h) = img.dimensions();
    let mut out = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    image::ImageEncoder::write_image(encoder, img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| TextureError::Png(e.to_string()))?;
    Ok(Converted {
        bytes: out,
        extension: "png",
        // X-Plane compresses PNGs as it loads them, to about a byte a pixel.
        vram_bytes: w as usize * h as usize * 4 / 3,
    })
}

/// Shrink RGBA8 pixels to fit `max_side`, or to exactly `size` when given.
fn shrink(rgba: Vec<u8>, w: u32, h: u32, max_side: u32, size: Option<(u32, u32)>) -> (u32, u32, Vec<u8>) {
    let target = size.unwrap_or_else(|| {
        if w.max(h) <= max_side {
            (w, h)
        } else {
            let k = max_side as f64 / w.max(h) as f64;
            (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1))
        }
    });
    if target == (w, h) {
        return (w, h, rgba);
    }
    match image::RgbaImage::from_raw(w, h, rgba) {
        Some(img) => {
            let out = image::imageops::resize(&img, target.0, target.1, image::imageops::FilterType::Triangle);
            (target.0, target.1, out.into_raw())
        }
        None => (target.0, target.1, vec![0; (target.0 * target.1 * 4) as usize]),
    }
}

/// Decode any texture to RGBA8, top row first, at most `max_side` on its
/// longer side (from a smaller mip level where there is one).
pub fn decode_within(data: &[u8], max_side: u32) -> Result<(u32, u32, Vec<u8>), TextureError> {
    if detect(data) == SourceFormat::Png {
        let img = image::load_from_memory_with_format(data, image::ImageFormat::Png)
            .map_err(|e| TextureError::Png(e.to_string()))?
            .to_rgba8();
        let (w, h) = img.dimensions();
        return Ok(shrink(img.into_raw(), w, h, max_side, None));
    }
    let img = load(data)?;
    let small = from_level(&img, first_level_within(&img, max_side, false));
    let rgba = decode_rgba8(&small, 0)?;
    Ok(shrink(rgba, small.width, small.height, max_side, None))
}

/// An X-Plane normal map in its NORMAL_METALNESS layout, from an MSFS normal
/// map and, when there is one, its occlusion/roughness/metalness texture.
/// Red and green carry the normal; green is inverted, because MSFS packs
/// DirectX-style normals (green pointing down the image) and X-Plane, whose
/// images the converter stores flipped, reads green as up. Blue is the
/// metalness and alpha the smoothness (X-Plane: white is smooth; MSFS keeps
/// roughness in green). X-Plane wants normal maps uncompressed, as PNG.
pub fn normal_metal_png(normal: &[u8], comp: Option<&[u8]>, max_side: u32) -> Result<Converted, TextureError> {
    let (w, h, n) = decode_within(normal, max_side)?;
    let comp = match comp {
        Some(c) => {
            let (cw, ch, c) = decode_within(c, max_side)?;
            Some(shrink(c, cw, ch, max_side, Some((w, h))).2)
        }
        None => None,
    };
    let mut out = vec![0u8; (w * h * 4) as usize];
    for (i, px) in out.chunks_exact_mut(4).enumerate() {
        let (r, g) = (n[i * 4], n[i * 4 + 1]);
        let (metal, smooth) = comp.as_ref().map_or((0, 128), |c| (c[i * 4 + 2], 255 - c[i * 4 + 1]));
        px.copy_from_slice(&[r, 255 - g, metal, smooth]);
    }
    let mut bytes = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    image::ImageEncoder::write_image(encoder, &out, w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| TextureError::Png(e.to_string()))?;
    Ok(Converted {
        bytes,
        extension: "png",
        // Uncompressed, four bytes a pixel, plus mips.
        vram_bytes: w as usize * h as usize * 4 * 4 / 3,
    })
}

/// Convert any MSFS texture, keeping its longer side at most `max_side`.
/// Block-compressed textures drop their largest mip levels, which costs
/// nothing beyond the lower resolution; everything else is resized.
pub fn convert_for_xplane_capped(data: &[u8], max_side: u32) -> Result<Converted, TextureError> {
    if detect(data) == SourceFormat::Png {
        // Size from the IHDR chunk; a PNG that fits, or whose size cannot be
        // read, passes through untouched.
        let dims = (data.len() >= 24).then(|| {
            (
                u32::from_be_bytes([data[16], data[17], data[18], data[19]]),
                u32::from_be_bytes([data[20], data[21], data[22], data[23]]),
            )
        });
        let (w, h) = match dims {
            Some((w, h)) if w.max(h) > max_side => (w, h),
            _ => {
                return Ok(Converted {
                    bytes: data.to_vec(),
                    extension: "png",
                    vram_bytes: dims.map_or(0, |(w, h)| w as usize * h as usize * 4 / 3),
                })
            }
        };
        let img = image::load_from_memory_with_format(data, image::ImageFormat::Png)
            .map_err(|e| TextureError::Png(e.to_string()))?
            .to_rgba8();
        return png_within(img.into_raw(), w, h, max_side);
    }
    let img = load(data)?;
    if writes_dds(&img) {
        let small = from_level(&img, first_level_within(&img, max_side, true));
        let bytes = to_xplane_dds(&small)?;
        let vram_bytes = bytes.len().saturating_sub(128);
        return Ok(Converted {
            bytes,
            extension: "dds",
            vram_bytes,
        });
    }
    let small = from_level(&img, first_level_within(&img, max_side, false));
    let rgba = decode_rgba8(&small, 0)?;
    png_within(rgba, small.width, small.height, max_side)
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

    #[test]
    fn signed_bc5_decodes_to_normal_map_values() {
        // Red: endpoints +127 and -127, every pixel index 0, so +1 (255).
        // Green: both endpoints 0, so 0 (the middle, 128).
        let block = [0x7F, 0x81, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let img = TextureImage {
            width: 4,
            height: 4,
            format: PixelFormat::Bc5s,
            mips: vec![block.to_vec()],
        };
        let px = decode_rgba8(&img, 0).unwrap();
        assert_eq!(&px[0..4], &[255, 128, 0, 255]);
        assert!(px.chunks(4).all(|p| p == [255, 128, 0, 255]));
        let unsigned = TextureImage {
            format: PixelFormat::Bc5,
            ..img
        };
        assert_eq!(&decode_rgba8(&unsigned, 0).unwrap()[0..2], &[127, 0]);
    }

    #[test]
    fn normal_maps_take_the_x_plane_channel_layout() {
        // One 4x4 level each: normal (R 255, G 128), and a COMP texture
        // (occlusion 10, roughness 200, metalness 100).
        let normal = ktx2(37, 4, 4, &[[255u8, 128, 0, 255].repeat(16)], 0);
        let comp = ktx2(37, 4, 4, &[[10u8, 200, 100, 255].repeat(16)], 0);
        let c = normal_metal_png(&normal, Some(&comp), 1024).unwrap();
        let img = image::load_from_memory_with_format(&c.bytes, image::ImageFormat::Png).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0).0, [255, 127, 100, 55], "green inverted, metal in blue, smoothness in alpha");
        let bare = normal_metal_png(&normal, None, 1024).unwrap();
        let img = image::load_from_memory_with_format(&bare.bytes, image::ImageFormat::Png).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0).0, [255, 127, 0, 128]);
    }

    fn mirrored(px: &[u8], w: usize, h: usize) -> Vec<u8> {
        (0..h)
            .flat_map(|y| (0..w).rev().flat_map(move |x| (0..4).map(move |c| (y, x, c))))
            .map(|(y, x, c)| px[(y * w + x) * 4 + c])
            .collect()
    }

    #[test]
    fn block_mirroring_matches_a_pixel_mirror() {
        for (fmt, seed) in [(PixelFormat::Bc1, 7), (PixelFormat::Bc2, 8), (PixelFormat::Bc3, 9)] {
            let (w, h) = (16u32, 8u32);
            let img = TextureImage {
                width: w,
                height: h,
                format: fmt,
                mips: vec![noise(level_size(fmt, w, h), seed)],
            };
            let before = decode_rgba8(&img, 0).unwrap();
            let flipped = TextureImage {
                mips: vec![mirror_bc_level(fmt, &img.mips[0], w, h).unwrap()],
                ..img
            };
            let after = decode_rgba8(&flipped, 0).unwrap();
            assert_eq!(after, mirrored(&before, w as usize, h as usize), "{fmt:?}");
        }
    }

    #[test]
    fn converted_files_flip_both_ways() {
        let levels: Vec<Vec<u8>> = [(16, 4), (8, 5), (4, 6)]
            .iter()
            .map(|&(side, seed)| noise(level_size(PixelFormat::Bc3, side, side), seed))
            .collect();
        let dds = convert_for_xplane(&ktx2(137, 16, 16, &levels, 0)).unwrap().bytes;
        let px = |bytes: &[u8]| decode_rgba8(&load_dds(bytes).unwrap(), 0).unwrap();
        let before = px(&dds);
        let turned = transform_texture_file(&dds, true, true).unwrap();
        let back = transform_texture_file(&turned, true, true).unwrap();
        assert_eq!(px(&back), before, "turning twice gives the original back");
        let h = transform_texture_file(&dds, false, true).unwrap();
        assert_eq!(px(&h), mirrored(&before, 16, 16));
        assert_eq!(u32::from_le_bytes(h[28..32].try_into().unwrap()), 3, "all three levels kept");

        let img = image::RgbaImage::from_raw(2, 1, vec![1, 2, 3, 255, 9, 9, 9, 255]).unwrap();
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let out = transform_texture_file(&png, false, true).unwrap();
        let back = image::load_from_memory(&out).unwrap().to_rgba8();
        assert_eq!(back.get_pixel(0, 0).0, [9, 9, 9, 255]);
    }

    #[test]
    fn capped_textures_drop_their_largest_levels() {
        let levels: Vec<Vec<u8>> = [(16, 1), (8, 2), (4, 3)]
            .iter()
            .map(|&(side, seed)| noise(level_size(PixelFormat::Bc1, side, side), seed))
            .collect();
        let data = ktx2(131, 16, 16, &levels, 0);
        let full = convert_for_xplane(&data).unwrap();
        let small = convert_for_xplane_capped(&data, 8).unwrap();
        assert_eq!((full.extension, small.extension), ("dds", "dds"));
        assert_eq!(output_extension(&data).unwrap(), "dds");
        let dims = |b: &[u8]| {
            let at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
            (at(16), at(12), at(28))
        };
        assert_eq!(dims(&full.bytes), (16, 16, 3));
        assert_eq!(dims(&small.bytes), (8, 8, 2), "the 16 px level is dropped");
        assert!(small.vram_bytes < full.vram_bytes);
    }

    #[test]
    fn capped_uncompressed_textures_are_resized() {
        let data = ktx2(37, 8, 4, &[noise(8 * 4 * 4, 5)], 0);
        let c = convert_for_xplane_capped(&data, 4).unwrap();
        assert_eq!(c.extension, "png");
        let img = image::load_from_memory_with_format(&c.bytes, image::ImageFormat::Png).unwrap();
        assert_eq!((img.width(), img.height()), (4, 2));
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
