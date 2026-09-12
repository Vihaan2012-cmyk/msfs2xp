//! Writing X-Plane DSF files that place objects.
//!
//! A DSF is a tree of "atoms" (a 4-character id and a length), wrapped in a
//! `XPLNEDSF` header and closed by an MD5 of everything before it. For an
//! overlay that only places objects, four atoms matter:
//!
//! - `HEAD/PROP`: the tile bounds and flags, as name/value string pairs;
//! - `DEFN/OBJT`: the object file paths, indexed from zero;
//! - `GEOD/POOL` + `SCAL`: point pools of (longitude, latitude, heading),
//!   stored as 16-bit integers with a per-plane scale and offset;
//! - `CMDS`: select a pool, select an object, place points.
//!
//! Precision is the subtle part. A 16-bit plane spanning a whole 1-degree tile
//! quantises latitude to about 1.7 m, which visibly misplaces buildings. Points
//! are therefore grouped into small cells, each with its own pool scaled to the
//! cell's extent, which brings the quantisation down to centimetres.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use md5::{Digest, Md5};

/// One object placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub lat: f64,
    pub lon: f64,
    /// Degrees true, clockwise from north.
    pub heading: f64,
    /// Index into the object path list.
    pub object: usize,
}

/// Cell size used to group points into pools, in degrees.
const CELL_DEG: f64 = 0.05;

/// The 1x1 degree tile containing a point, as (south, west).
pub fn tile_of(lat: f64, lon: f64) -> (i32, i32) {
    (lat.floor() as i32, lon.floor() as i32)
}

/// X-Plane's path for a tile: `+20+050/+25+055.dsf`.
pub fn tile_path(south: i32, west: i32) -> PathBuf {
    let dir_s = south.div_euclid(10) * 10;
    let dir_w = west.div_euclid(10) * 10;
    Path::new(&format!("{dir_s:+03}{dir_w:+04}")).join(format!("{south:+03}{west:+04}.dsf"))
}

fn atom(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    // Atom ids are C multi-character constants written little-endian, so the
    // characters appear reversed in the file.
    out.extend_from_slice(&[id[3], id[2], id[1], id[0]]);
    out.extend_from_slice(&((payload.len() + 8) as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn string_table(strings: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for s in strings {
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }
    out
}

/// Quantise a value into a 16-bit plane with the given offset and scale.
fn quantise(v: f64, offset: f64, scale: f64) -> u16 {
    if scale <= 0.0 {
        return 0;
    }
    ((v - offset) / scale * 65_535.0).round().clamp(0.0, 65_535.0) as u16
}

/// A plane's (scale, offset) covering `[min, max]`.
///
/// SCAL stores both as 32-bit floats, and X-Plane decodes with exactly those
/// values. An offset like 55.36 degrees is only good to about 0.4 m in f32, so
/// quantising against the unrounded value would shift every point. Instead the
/// offset is rounded down to an f32 (keeping every value at or above it) and the
/// scale rounded up to one (keeping the maximum in range), and points are then
/// quantised against those exact f32 values.
fn plane_range(min: f64, max: f64) -> (f64, f64) {
    let mut offset = min as f32;
    if (offset as f64) > min {
        offset = offset.next_down();
    }
    // X-Plane decodes a point as raw / 65535 * scale + offset, so the scale
    // is the plane's full span (confirmed against Laminar's own DSFs, whose
    // heading plane has scale 360).
    let span = (max - offset as f64).max(1e-9);
    let mut scale = span as f32;
    if (scale as f64) + (offset as f64) < max {
        scale = scale.next_up();
    }
    (scale as f64, offset as f64)
}

/// Build one DSF tile's bytes.
pub fn build_tile(south: i32, west: i32, objects: &[String], placements: &[Placement], agent: &str) -> Vec<u8> {
    // Group placements into cells, then each cell into chunks of at most
    // 65 535 points (the pool index limit).
    let mut cells: BTreeMap<(i64, i64), Vec<Placement>> = BTreeMap::new();
    for p in placements {
        let key = ((p.lat / CELL_DEG).floor() as i64, (p.lon / CELL_DEG).floor() as i64);
        cells.entry(key).or_default().push(*p);
    }
    let mut pools: Vec<Vec<Placement>> = Vec::new();
    for (_, mut list) in cells {
        // Sorting by object lets commands place runs of the same object.
        list.sort_by_key(|p| p.object);
        for chunk in list.chunks(65_535) {
            pools.push(chunk.to_vec());
        }
    }

    let props: Vec<String> = [
        ("sim/west", west.to_string()),
        ("sim/east", (west + 1).to_string()),
        ("sim/south", south.to_string()),
        ("sim/north", (south + 1).to_string()),
        ("sim/planet", "earth".to_string()),
        ("sim/overlay", "1".to_string()),
        ("sim/creation_agent", agent.to_string()),
        // Draw every object regardless of the user's object density setting.
        ("sim/require_object", "1/0".to_string()),
    ]
    .into_iter()
    .flat_map(|(k, v)| [k.to_string(), v])
    .collect();

    let head = atom(b"HEAD", &atom(b"PROP", &string_table(&props)));
    let mut defn = Vec::new();
    defn.extend(atom(b"TERT", &[]));
    defn.extend(atom(b"OBJT", &string_table(objects)));
    defn.extend(atom(b"POLY", &[]));
    defn.extend(atom(b"NETW", &[]));
    defn.extend(atom(b"DEMN", &[]));
    let defn = atom(b"DEFN", &defn);

    let mut geod = Vec::new();
    let mut cmds = Vec::new();
    for (pool_index, pool) in pools.iter().enumerate() {
        let (mut lat0, mut lat1, mut lon0, mut lon1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in pool {
            lat0 = lat0.min(p.lat);
            lat1 = lat1.max(p.lat);
            lon0 = lon0.min(p.lon);
            lon1 = lon1.max(p.lon);
        }
        let planes = [plane_range(lon0, lon1), plane_range(lat0, lat1), (360.0, 0.0)];
        let mut data = Vec::with_capacity(5 + pool.len() * 6 + 3);
        data.extend_from_slice(&(pool.len() as u32).to_le_bytes());
        data.push(3); // planes
        for (k, &(scale, offset)) in planes.iter().enumerate() {
            data.push(0); // raw encoding
            for p in pool {
                let v = match k {
                    0 => p.lon,
                    1 => p.lat,
                    _ => p.heading.rem_euclid(360.0),
                };
                data.extend_from_slice(&quantise(v, offset, scale).to_le_bytes());
            }
        }
        geod.extend(atom(b"POOL", &data));
        let mut scal = Vec::with_capacity(24);
        for (scale, offset) in planes {
            scal.extend_from_slice(&(scale as f32).to_le_bytes());
            scal.extend_from_slice(&(offset as f32).to_le_bytes());
        }
        geod.extend(atom(b"SCAL", &scal));

        // Select the pool, then place runs of the same object with ranges.
        cmds.push(1);
        cmds.extend_from_slice(&(pool_index as u16).to_le_bytes());
        let mut i = 0;
        while i < pool.len() {
            let obj = pool[i].object;
            let mut j = i;
            while j < pool.len() && pool[j].object == obj {
                j += 1;
            }
            if obj <= u8::MAX as usize {
                cmds.push(3);
                cmds.push(obj as u8);
            } else if obj <= u16::MAX as usize {
                cmds.push(4);
                cmds.extend_from_slice(&(obj as u16).to_le_bytes());
            } else {
                cmds.push(5);
                cmds.extend_from_slice(&(obj as u32).to_le_bytes());
            }
            cmds.push(8); // object range: first, last + 1
            cmds.extend_from_slice(&(i as u16).to_le_bytes());
            cmds.extend_from_slice(&(j as u16).to_le_bytes());
            i = j;
        }
    }
    let geod = atom(b"GEOD", &geod);
    let dems = atom(b"DEMS", &[]);
    let cmds = atom(b"CMDS", &cmds);

    let mut out = Vec::new();
    out.extend_from_slice(b"XPLNEDSF");
    out.extend_from_slice(&1u32.to_le_bytes());
    for part in [head, defn, geod, dems, cmds] {
        out.extend_from_slice(&part);
    }
    let digest = Md5::digest(&out);
    out.extend_from_slice(&digest);
    out
}

/// Split placements by tile and build every tile, returning `(relative path, bytes)`.
pub fn build_tiles(objects: &[String], placements: &[Placement], agent: &str) -> Vec<(PathBuf, Vec<u8>)> {
    let mut by_tile: BTreeMap<(i32, i32), Vec<Placement>> = BTreeMap::new();
    for p in placements {
        if p.lat.is_finite() && p.lon.is_finite() && p.object < objects.len() {
            by_tile.entry(tile_of(p.lat, p.lon)).or_default().push(*p);
        }
    }
    by_tile
        .into_iter()
        .map(|((s, w), list)| (tile_path(s, w), build_tile(s, w, objects, &list, agent)))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal DSF reader for tests: the atoms it finds and the placements
    /// it decodes, so the writer is checked against the format, not itself.
    pub(crate) struct Parsed {
        pub props: Vec<(String, String)>,
        pub objects: Vec<String>,
        pub placements: Vec<(f64, f64, f64, usize)>,
    }

    fn atoms(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos + 8 <= data.len() {
            let id = [data[pos + 3], data[pos + 2], data[pos + 1], data[pos]];
            let len = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
            assert!(len >= 8 && pos + len <= data.len(), "atom length out of range");
            out.push((id, &data[pos + 8..pos + len]));
            pos += len;
        }
        assert_eq!(pos, data.len(), "atoms must tile their parent exactly");
        out
    }

    fn strings(b: &[u8]) -> Vec<String> {
        b.split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).to_string())
            .collect()
    }

    pub(crate) fn parse(file: &[u8]) -> Parsed {
        assert_eq!(&file[..8], b"XPLNEDSF");
        let body = &file[12..file.len() - 16];
        let digest = Md5::digest(&file[..file.len() - 16]);
        assert_eq!(&file[file.len() - 16..], digest.as_slice(), "MD5 footer");
        let top = atoms(body);
        let find = |id: &[u8; 4]| top.iter().find(|(i, _)| i == id).map(|(_, b)| *b).unwrap();
        let prop = atoms(find(b"HEAD"))[0].1;
        let kv = strings(prop);
        let props = kv.chunks(2).map(|c| (c[0].clone(), c[1].clone())).collect();
        let defn = atoms(find(b"DEFN"));
        let objects = strings(defn.iter().find(|(i, _)| i == b"OBJT").unwrap().1);
        let geod = atoms(find(b"GEOD"));
        let mut pools: Vec<Vec<[f64; 3]>> = Vec::new();
        let mut raw_pools: Vec<(usize, Vec<Vec<u16>>)> = Vec::new();
        for (id, b) in &geod {
            if id == b"POOL" {
                let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
                let planes = b[4] as usize;
                let mut at = 5;
                let mut vals = Vec::new();
                for _ in 0..planes {
                    assert_eq!(b[at], 0, "raw encoding");
                    at += 1;
                    vals.push((0..n).map(|k| u16::from_le_bytes([b[at + 2 * k], b[at + 2 * k + 1]])).collect());
                    at += 2 * n;
                }
                raw_pools.push((n, vals));
            } else if id == b"SCAL" {
                let (n, vals) = raw_pools.last().unwrap();
                let f = |k: usize| f32::from_le_bytes([b[4 * k], b[4 * k + 1], b[4 * k + 2], b[4 * k + 3]]) as f64;
                pools.push(
                    (0..*n)
                        .map(|i| {
                            [
                                vals[0][i] as f64 / 65_535.0 * f(0) + f(1),
                                vals[1][i] as f64 / 65_535.0 * f(2) + f(3),
                                vals[2][i] as f64 / 65_535.0 * f(4) + f(5),
                            ]
                        })
                        .collect(),
                );
            }
        }
        let cmds = find(b"CMDS");
        let (mut pool, mut def, mut at) = (0usize, 0usize, 0usize);
        let mut placements = Vec::new();
        while at < cmds.len() {
            match cmds[at] {
                1 => {
                    pool = u16::from_le_bytes([cmds[at + 1], cmds[at + 2]]) as usize;
                    at += 3;
                }
                3 => {
                    def = cmds[at + 1] as usize;
                    at += 2;
                }
                4 => {
                    def = u16::from_le_bytes([cmds[at + 1], cmds[at + 2]]) as usize;
                    at += 3;
                }
                8 => {
                    let a = u16::from_le_bytes([cmds[at + 1], cmds[at + 2]]) as usize;
                    let b = u16::from_le_bytes([cmds[at + 3], cmds[at + 4]]) as usize;
                    for p in &pools[pool][a..b] {
                        placements.push((p[1], p[0], p[2], def));
                    }
                    at += 5;
                }
                other => panic!("unexpected command {other}"),
            }
        }
        Parsed {
            props,
            objects,
            placements,
        }
    }

    #[test]
    fn tile_paths_follow_xplane_naming() {
        assert_eq!(tile_path(25, 55), Path::new("+20+050").join("+25+055.dsf"));
        assert_eq!(tile_path(41, -88), Path::new("+40-090").join("+41-088.dsf"));
        assert_eq!(tile_path(-34, 151), Path::new("-40+150").join("-34+151.dsf"));
        assert_eq!(tile_of(-33.9, 151.2), (-34, 151));
    }

    #[test]
    fn round_trips_placements_to_centimetres() {
        let objects = vec!["objects/a.obj".to_string(), "objects/b.obj".to_string()];
        let placements = vec![
            Placement {
                lat: 25.252_811,
                lon: 55.364_402,
                heading: 211.97,
                object: 1,
            },
            Placement {
                lat: 25.260_004,
                lon: 55.345_123,
                heading: 0.0,
                object: 0,
            },
            Placement {
                lat: 25.249_999,
                lon: 55.380_001,
                heading: 359.5,
                object: 1,
            },
        ];
        let tile = build_tile(25, 55, &objects, &placements, "test");
        let parsed = parse(&tile);
        assert_eq!(parsed.objects, objects);
        assert!(parsed.props.contains(&("sim/west".into(), "55".into())));
        assert!(parsed.props.contains(&("sim/overlay".into(), "1".into())));
        assert_eq!(parsed.placements.len(), 3);
        for p in &placements {
            let got = parsed
                .placements
                .iter()
                .find(|q| q.3 == p.object && (q.0 - p.lat).abs() < 1e-5 && (q.1 - p.lon).abs() < 1e-5)
                .unwrap_or_else(|| panic!("placement {p:?} missing"));
            let err_m = ((got.0 - p.lat) * 110_540.0).hypot((got.1 - p.lon) * 100_000.0);
            assert!(err_m < 0.02, "position error {err_m} m");
            assert!((got.2 - p.heading).abs() < 0.01, "heading {} vs {}", got.2, p.heading);
        }
    }

    #[test]
    fn planes_are_exact_in_f32_and_cover_the_range() {
        for (min, max) in [(55.345_123, 55.380_001), (-87.95, -87.86), (25.249_999, 25.249_999)] {
            let (scale, offset) = plane_range(min, max);
            assert_eq!(offset, offset as f32 as f64, "offset must be an exact f32");
            assert_eq!(scale, scale as f32 as f64, "scale must be an exact f32");
            assert!(offset <= min, "offset {offset} above min {min}");
            assert!(offset + 65_535.0 * scale >= max, "range does not reach max");
        }
    }

    #[test]
    fn placements_are_split_across_tiles() {
        let objects = vec!["objects/a.obj".to_string()];
        let placements = vec![
            Placement {
                lat: 41.99,
                lon: -87.91,
                heading: 0.0,
                object: 0,
            },
            Placement {
                lat: 42.01,
                lon: -87.91,
                heading: 0.0,
                object: 0,
            },
        ];
        let tiles = build_tiles(&objects, &placements, "test");
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].0, Path::new("+40-090").join("+41-088.dsf"));
        assert_eq!(parse(&tiles[1].1).placements.len(), 1);
    }

    #[test]
    fn many_objects_use_wider_definition_commands() {
        let objects: Vec<String> = (0..300).map(|i| format!("objects/o{i}.obj")).collect();
        let placements: Vec<Placement> = (0..300)
            .map(|i| Placement {
                lat: 25.25 + i as f64 * 1e-5,
                lon: 55.36,
                heading: 90.0,
                object: i,
            })
            .collect();
        let parsed = parse(&build_tile(25, 55, &objects, &placements, "test"));
        assert_eq!(parsed.placements.len(), 300);
        assert!(parsed.placements.iter().any(|p| p.3 == 299));
    }
}
