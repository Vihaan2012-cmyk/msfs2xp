//! MSFS 2024 SimProp container files (`.spb`).
//!
//! MSFS 2024 airports group buildings and their props into "SimProp
//! containers". A scenery record (0x1B) places a container; the container file
//! lists the library models it is made of, each with an offset and orientation
//! relative to the container.
//!
//! The file is compiled SimPropBinary. After a small header comes a table of
//! 20-byte entries: a property GUID and the size of its value, or 0xFFFFFFFF
//! for variable length. The body is a stream of tagged values, where the tag is
//! a 1-based index into that table, variable-length values carry a u32 length,
//! and tag 0 closes a set. Property meaning comes from the GUID, never the
//! index, because each file numbers its own table. The GUIDs are the ones in
//! the simulator's propdefs (`propsimpropcontainer.xml`, `propworldbase.xml`).
//!
//! Offsets are in the simulator's frame: +X right, +Y up, +Z forward, in
//! metres, relative to the container's position and heading. Orientation is a
//! PBH triple of 32-bit fractions of a full turn.

use crate::bgl::guid::Guid;

/// Parse "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" into the on-disk (Windows,
/// mixed-endian) byte order.
const fn guid(s: &str) -> [u8; 16] {
    const fn hex(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => panic!("bad hex digit"),
        }
    }
    let b = s.as_bytes();
    let pos = [0, 2, 4, 6, 9, 11, 14, 16, 19, 21, 24, 26, 28, 30, 32, 34];
    let mut canon = [0u8; 16];
    let mut i = 0;
    while i < 16 {
        canon[i] = (hex(b[pos[i]]) << 4) | hex(b[pos[i] + 1]);
        i += 1;
    }
    [
        canon[3], canon[2], canon[1], canon[0], canon[5], canon[4], canon[7], canon[6], canon[8], canon[9],
        canon[10], canon[11], canon[12], canon[13], canon[14], canon[15],
    ]
}

/// One child object placed by a container.
pub const SIM_PROP_ATTACH: [u8; 16] = guid("ad124d80-114c-4682-9bd0-783fb99c5023");
pub const OFFSET_XYZ: [u8; 16] = guid("b975cd65-7cef-4cc2-9ab1-24b7cfefa02c");
pub const ORIENTATION: [u8; 16] = guid("fbedc683-8576-4138-b70b-383231f6132a");
pub const MDL_GUID: [u8; 16] = guid("8588e41e-89ca-47f9-8210-f8561d93d17c");
pub const SCALE: [u8; 16] = guid("0119970b-fc6f-4979-b416-8e664d97fc54");

const MAGIC: [u8; 2] = [0xAC, 0xEB];
const TABLE_AT: usize = 0x32;
const VARIABLE: u32 = 0xFFFF_FFFF;

/// A library model placed by a container, relative to the container.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerChild {
    pub model: Guid,
    /// Metres: +X right, +Y up, +Z forward.
    pub offset: [f32; 3],
    /// Degrees.
    pub pitch: f32,
    pub bank: f32,
    pub heading: f32,
    pub scale: f32,
}

enum Node<'a> {
    Value { ty: [u8; 16], bytes: &'a [u8] },
    Set { ty: [u8; 16], children: Vec<Node<'a>> },
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Parse a byte range as a list of tagged values. `None` when it is not one
/// (the caller then treats the range as an opaque value, such as text).
fn parse_list<'a>(data: &'a [u8], types: &[([u8; 16], u32)], depth: usize) -> Option<Vec<Node<'a>>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let tag = u32_at(data, at)? as usize;
        at += 4;
        if tag == 0 {
            continue; // end of a set
        }
        let (ty, size) = *types.get(tag - 1)?;
        if size == VARIABLE {
            let len = u32_at(data, at)? as usize;
            at += 4;
            let bytes = data.get(at..at.checked_add(len)?)?;
            at += len;
            let nested = if depth < 32 { parse_list(bytes, types, depth + 1) } else { None };
            out.push(match nested {
                Some(children) if !children.is_empty() => Node::Set { ty, children },
                _ => Node::Value { ty, bytes },
            });
        } else {
            let bytes = data.get(at..at.checked_add(size as usize)?)?;
            at += size as usize;
            out.push(Node::Value { ty, bytes });
        }
    }
    Some(out)
}

fn angle(v: u32) -> f32 {
    (v as f64 * 360.0 / 4_294_967_296.0) as f32
}

fn signed(a: f32) -> f32 {
    if a > 180.0 {
        a - 360.0
    } else {
        a
    }
}

/// Fill `child` from the values anywhere below one SimPropAttach set.
fn collect(nodes: &[Node], child: &mut ContainerChild, found_model: &mut bool) {
    for n in nodes {
        match n {
            Node::Set { children, .. } => collect(children, child, found_model),
            Node::Value { ty, bytes } => {
                if *ty == MDL_GUID {
                    if let Some(g) = Guid::from_slice(bytes).filter(|g| !g.is_nil()) {
                        child.model = g;
                        *found_model = true;
                    }
                } else if *ty == OFFSET_XYZ && bytes.len() >= 12 {
                    let f = |k: usize| f32::from_le_bytes([bytes[k], bytes[k + 1], bytes[k + 2], bytes[k + 3]]);
                    let v = [f(0), f(4), f(8)];
                    if v.iter().all(|x| x.is_finite()) {
                        child.offset = v;
                    }
                } else if *ty == ORIENTATION && bytes.len() >= 12 {
                    if let (Some(p), Some(b), Some(h)) = (u32_at(bytes, 0), u32_at(bytes, 4), u32_at(bytes, 8)) {
                        child.pitch = signed(angle(p));
                        child.bank = signed(angle(b));
                        child.heading = angle(h);
                    }
                } else if *ty == SCALE && bytes.len() >= 4 {
                    let s = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                    if s.is_finite() && s > 0.0 {
                        child.scale = s;
                    }
                }
            }
        }
    }
}

fn find_children(nodes: &[Node], out: &mut Vec<ContainerChild>) {
    for n in nodes {
        if let Node::Set { ty, children } = n {
            if *ty == SIM_PROP_ATTACH {
                let mut child = ContainerChild {
                    model: Guid::NIL,
                    offset: [0.0; 3],
                    pitch: 0.0,
                    bank: 0.0,
                    heading: 0.0,
                    scale: 1.0,
                };
                let mut found = false;
                collect(children, &mut child, &mut found);
                if found {
                    out.push(child);
                }
            } else {
                find_children(children, out);
            }
        }
    }
}

/// Read the library models placed by a container file. Lights and other
/// non-model children are skipped.
pub fn parse_container(data: &[u8]) -> Result<Vec<ContainerChild>, String> {
    if data.len() < TABLE_AT || data[..2] != MAGIC {
        return Err("not a SimPropBinary file".into());
    }
    let count = u32_at(data, 0x1A).ok_or("truncated header")? as usize;
    let entries = count.saturating_sub(1);
    let body = TABLE_AT + 20 * entries;
    if entries > 4096 || body > data.len() {
        return Err(format!("property table of {entries} entries does not fit"));
    }
    let types: Vec<([u8; 16], u32)> = (0..entries)
        .map(|i| {
            let at = TABLE_AT + 20 * i;
            let mut g = [0u8; 16];
            g.copy_from_slice(&data[at..at + 16]);
            (g, u32_at(data, at + 16).unwrap_or(VARIABLE))
        })
        .collect();
    let nodes = parse_list(&data[body..], &types, 0).ok_or("malformed property stream")?;
    let mut out = Vec::new();
    find_children(&nodes, &mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: [u8; 16] = guid("ee6fd77b-14bb-497c-ae59-0ccdd90eaa68");
    const LIBRARY_OBJECT: [u8; 16] = guid("75678bb8-2c3b-4124-a396-b1722e093b92");
    const DISPLAY_NAME: [u8; 16] = guid("569e843a-4328-4246-ac4f-d13224582dd3");
    const MODEL: [u8; 16] = [7; 16];

    /// Build a container file: header, table, then the body.
    fn file(types: &[([u8; 16], u32)], body: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; TABLE_AT];
        out[0] = 0xAC;
        out[1] = 0xEB;
        out[0x1A..0x1E].copy_from_slice(&((types.len() + 1) as u32).to_le_bytes());
        for (g, size) in types {
            out.extend_from_slice(g);
            out.extend_from_slice(&size.to_le_bytes());
        }
        out.extend_from_slice(body);
        out
    }

    fn var(tag: u32, payload: &[u8]) -> Vec<u8> {
        let mut v = tag.to_le_bytes().to_vec();
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn fixed(tag: u32, payload: &[u8]) -> Vec<u8> {
        let mut v = tag.to_le_bytes().to_vec();
        v.extend_from_slice(payload);
        v
    }

    fn close(mut v: Vec<u8>) -> Vec<u8> {
        v.extend_from_slice(&0u32.to_le_bytes());
        v
    }

    #[test]
    fn guid_constants_use_disk_byte_order() {
        // {EE6FD77B-14BB-497C-AE59-0CCDD90EAA68} as it appears in real files.
        assert_eq!(DOCUMENT[..6], [0x7B, 0xD7, 0x6F, 0xEE, 0xBB, 0x14]);
    }

    #[test]
    fn reads_a_child_with_offset_orientation_and_scale() {
        let types = [
            (DOCUMENT, VARIABLE),        // 1
            (SIM_PROP_ATTACH, VARIABLE), // 2
            (DISPLAY_NAME, VARIABLE),    // 3
            (OFFSET_XYZ, 16),            // 4
            (ORIENTATION, 16),           // 5
            (LIBRARY_OBJECT, VARIABLE),  // 6
            (MDL_GUID, 16),              // 7
            (SCALE, 4),                  // 8
        ];
        let mut offset = Vec::new();
        for f in [-498.25f32, 9.641, -156.149, 0.0] {
            offset.extend_from_slice(&f.to_le_bytes());
        }
        let mut pbh = Vec::new();
        for v in [0u32, 0, 0x4000_0000, 0] {
            pbh.extend_from_slice(&v.to_le_bytes()); // heading a quarter turn
        }
        let mut lib = fixed(7, &MODEL);
        lib.extend(fixed(8, &1.5f32.to_le_bytes()));
        let mut attach = var(3, b"opaque name bytes");
        attach.extend(fixed(4, &offset));
        attach.extend(fixed(5, &pbh));
        attach.extend(var(6, &close(lib)));
        let body = var(1, &close(var(2, &close(attach))));
        let children = parse_container(&file(&types, &body)).unwrap();
        assert_eq!(children.len(), 1);
        let c = &children[0];
        assert_eq!(c.model, Guid(MODEL));
        assert_eq!(c.offset, [-498.25, 9.641, -156.149]);
        assert!((c.heading - 90.0).abs() < 1e-4);
        assert_eq!(c.scale, 1.5);
    }

    #[test]
    fn a_child_without_offset_sits_at_the_container_origin() {
        let types = [(SIM_PROP_ATTACH, VARIABLE), (MDL_GUID, 16)];
        let body = var(1, &close(fixed(2, &MODEL)));
        let c = parse_container(&file(&types, &body)).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].offset, [0.0; 3]);
        assert_eq!(c[0].scale, 1.0);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(parse_container(b"nope").is_err());
        let types = [(SIM_PROP_ATTACH, VARIABLE)];
        let mut f = file(&types, &[]);
        f.extend_from_slice(&[9, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0x7F]);
        assert!(parse_container(&f).is_err());
    }
}
