//! MSFS apron decals as X-Plane draped polygons.
//!
//! MSFS paints markings such as hold-short bars, hatched areas, stop and arrow
//! stencils and tyre grime as textured apron polygons ("decals"). X-Plane's
//! apt.dat cannot draw textured pavement, but a DSF can drape a textured
//! polygon over it, with explicit per-vertex texture coordinates.
//!
//! How the MSFS texture lands on the polygon, checked against O'Hare:
//! - The texture's "up" points along the compass heading `-uv_rotation`: all
//!   16 runway-number decals point along their runway to 0.1 degrees.
//! - With flag bit 0x80 the texture is stretched once over the polygon, top
//!   edge on the "up" side (runway numbers read correctly from the approach,
//!   and the 60 ft numeral height comes out right).
//! - Without it the texture repeats every `uv_scale` metres from the polygon's
//!   corner, top edge on the side opposite "up": the dashed half of the
//!   hold-short texture then faces the runway, as the FAA pattern requires, for
//!   156 of 176 hold-short bars (the rest sit between two runways).
//!
//! Skipped: runway numbers (X-Plane paints its own) and the aircraft-type
//! label atlas (which label each piece shows could not be established).

use crate::geo::{LatLon, Plane};
use crate::model::Apron;

/// What kind of marking a decal is, which sets its draw layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecalKind {
    /// Tyre marks and dirt: drawn below the other markings.
    Grime,
    /// Painted markings and stencils.
    Marking,
}

/// One decal polygon ready for a DSF: `(lon, lat, s, t)` per vertex,
/// counter-clockwise, not closed.
#[derive(Debug, Clone, PartialEq)]
pub struct DecalPolygon {
    pub texture: String,
    pub stretched: bool,
    pub kind: DecalKind,
    pub priority: u32,
    pub points: Vec<(f64, f64, f64, f64)>,
}

/// Which decals to convert, and as what.
pub fn classify(material_name: &str) -> Option<DecalKind> {
    let n = material_name.to_ascii_lowercase();
    if n.contains("atlas") {
        return None;
    }
    // Runway designators ("INI_Decal_4R", "INI_Decal_28C"): X-Plane draws its own.
    let tail = n.rsplit('_').next().unwrap_or("");
    let designator = n.contains("decal_")
        && !tail.is_empty()
        && tail.len() <= 3
        && tail.chars().next().is_some_and(|c| c.is_ascii_digit())
        && tail.chars().all(|c| c.is_ascii_digit() || matches!(c, 'l' | 'r' | 'c'));
    if designator {
        return None;
    }
    // Painted markings first: "INI_Decal_Hatched_Dirty" is a hatched marking,
    // not grime, despite the word "dirty".
    if n.contains("decal") || n.contains("hatch") {
        return Some(DecalKind::Marking);
    }
    if ["dirt", "tire", "tyre", "grime", "stain", "skid"].iter().any(|w| n.contains(w)) {
        Some(DecalKind::Grime)
    } else {
        Some(DecalKind::Marking)
    }
}

/// Texture coordinates for a decal outline. `None` for fewer than three
/// distinct vertices or a degenerate texture frame.
pub fn texture_polygon(
    vertices: &[LatLon],
    uv_scale: f32,
    uv_rotation: f32,
    stretched: bool,
) -> Option<Vec<(f64, f64, f64, f64)>> {
    let mut v: Vec<LatLon> = Vec::with_capacity(vertices.len());
    for p in vertices {
        if v.last().is_none_or(|q: &LatLon| (q.lat - p.lat).abs() > 1e-9 || (q.lon - p.lon).abs() > 1e-9) {
            v.push(*p);
        }
    }
    if v.len() > 1 && (v[0].lat - v[v.len() - 1].lat).abs() < 1e-9 && (v[0].lon - v[v.len() - 1].lon).abs() < 1e-9 {
        v.pop();
    }
    if v.len() < 3 {
        return None;
    }
    let plane = Plane::new(v[0]);
    let xy: Vec<(f64, f64)> = v.iter().map(|p| plane.to_xy(*p)).collect();
    // "Up" is the compass heading -rotation; "right" is 90 degrees clockwise.
    let h = -(uv_rotation as f64);
    let up = (h.sin(), h.cos());
    let right = (h.cos(), -h.sin());
    let pu: Vec<f64> = xy.iter().map(|p| p.0 * up.0 + p.1 * up.1).collect();
    let pr: Vec<f64> = xy.iter().map(|p| p.0 * right.0 + p.1 * right.1).collect();
    let (umin, umax) = pu.iter().fold((f64::MAX, f64::MIN), |a, &x| (a.0.min(x), a.1.max(x)));
    let (rmin, rmax) = pr.iter().fold((f64::MAX, f64::MIN), |a, &x| (a.0.min(x), a.1.max(x)));
    let st: Vec<(f64, f64)> = if stretched {
        let (w, d) = (rmax - rmin, umax - umin);
        if w < 1e-3 || d < 1e-3 {
            return None;
        }
        // X-Plane's T runs up the image, so the top edge (T = 1) is "up".
        pr.iter().zip(&pu).map(|(&r, &u)| ((r - rmin) / w, (u - umin) / d)).collect()
    } else {
        let s = uv_scale as f64;
        if !(s.is_finite() && s > 0.01) {
            return None;
        }
        // Image rows run along "up", so the top edge sits at the "down" side.
        pr.iter().zip(&pu).map(|(&r, &u)| ((r - rmin) / s, 1.0 - (u - umin) / s)).collect()
    };
    let mut out: Vec<(f64, f64, f64, f64)> =
        v.iter().zip(&st).map(|(p, &(s, t))| (p.lon, p.lat, s, t)).collect();
    // X-Plane wants counter-clockwise exteriors.
    let area: f64 = (0..xy.len())
        .map(|i| {
            let (a, b) = (xy[i], xy[(i + 1) % xy.len()]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    if area < 0.0 {
        out.reverse();
    }
    Some(out)
}

/// A four-cornered outline whose sides are within 10% of each other in length
/// and whose corners are right angles (to 5 degrees): the shape MSFS gives a
/// decal meant to show its square texture exactly once.
pub fn is_square_quad(vertices: &[LatLon]) -> bool {
    let mut v: Vec<LatLon> = vertices.to_vec();
    if v.len() == 5 && (v[0].lat - v[4].lat).abs() < 1e-9 && (v[0].lon - v[4].lon).abs() < 1e-9 {
        v.pop();
    }
    if v.len() != 4 {
        return false;
    }
    let plane = Plane::new(v[0]);
    let p: Vec<(f64, f64)> = v.iter().map(|q| plane.to_xy(*q)).collect();
    let side = |i: usize| {
        let (a, b) = (p[i], p[(i + 1) % 4]);
        (b.0 - a.0, b.1 - a.1)
    };
    let lens: Vec<f64> = (0..4).map(|i| side(i).0.hypot(side(i).1)).collect();
    let (lo, hi) = lens.iter().fold((f64::MAX, f64::MIN), |a, &x| (a.0.min(x), a.1.max(x)));
    if lo < 1e-3 || hi / lo > 1.1 {
        return false;
    }
    (0..4).all(|i| {
        let (a, b) = (side(i), side((i + 1) % 4));
        let cos = (a.0 * b.0 + a.1 * b.1) / (lens[i] * lens[(i + 1) % 4]);
        cos.abs() < 0.087 // within 5 degrees of a right angle
    })
}

/// The decal polygons of an airport's aprons, lowest priority first.
pub fn airport_decals(aprons: &[Apron]) -> Vec<DecalPolygon> {
    let mut out = Vec::new();
    for a in aprons {
        if a.draw {
            continue;
        }
        let (Some(name), Some(texture)) = (a.material_name.as_deref(), a.decal_texture.as_deref()) else {
            continue;
        };
        let Some(kind) = classify(name) else { continue };
        // Flag 0x80 stretches the texture once only when the decal is a square
        // quad, matching its square texture (stencils, runway numbers, dirt
        // patches). Hatched areas carry the flag on long strips, and stretching
        // them turns fine hatching into two or three giant stripes, so those tile.
        let stretched = a.flags & 0x80 != 0 && is_square_quad(&a.vertices);
        // Tiled grime is cut off hard at the polygon edge, which MSFS feathers
        // and X-Plane cannot; stretched grime textures fade out on their own.
        if kind == DecalKind::Grime && !stretched {
            continue;
        }
        if let Some(points) = texture_polygon(&a.vertices, a.uv_scale, a.uv_rotation, stretched) {
            out.push(DecalPolygon {
                texture: texture.to_string(),
                stretched,
                kind,
                priority: a.priority,
                points,
            });
        }
    }
    out.sort_by_key(|d| (d.kind == DecalKind::Marking, d.priority));
    out
}

/// The `.pol` definition for a decal texture.
pub fn pol_text(texture_path: &str, stretched: bool, kind: DecalKind) -> String {
    let tex = if stretched { "TEXTURE_NOWRAP" } else { "TEXTURE" };
    // Below apt.dat painted lines (layer "markings" 0), above all pavement;
    // grime under the painted markings.
    let layer = match kind {
        DecalKind::Grime => -2,
        DecalKind::Marking => -1,
    };
    format!("A\n850\nDRAPED_POLYGON\n\n{tex} {texture_path}\nSCALE 25 25\nLAYER_GROUP markings {layer}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square `side` metres wide around a point, counter-clockwise from the SW corner.
    fn square(lat: f64, lon: f64, side: f64) -> Vec<LatLon> {
        let dlat = side / 110_574.0;
        let dlon = side / (111_320.0 * lat.to_radians().cos());
        vec![
            LatLon::new(lat, lon),
            LatLon::new(lat, lon + dlon),
            LatLon::new(lat + dlat, lon + dlon),
            LatLon::new(lat + dlat, lon),
        ]
    }

    #[test]
    fn classifies_markings_grime_and_skips() {
        assert_eq!(classify("INI_Decal_Holdshort"), Some(DecalKind::Marking));
        assert_eq!(classify("INI_Decal_Hatched_Thin"), Some(DecalKind::Marking));
        assert_eq!(classify("INI_Dirt_Turn90"), Some(DecalKind::Grime));
        assert_eq!(classify("INI_Decal_28C"), None, "runway numbers are X-Plane's");
        assert_eq!(classify("INI_Decal_4R"), None);
        assert_eq!(classify("KORD_ACF_ATLAS"), None);
        assert_eq!(classify("INI_Decal_20MPH_Box"), Some(DecalKind::Marking));
        assert_eq!(classify("INI_Decal_Hatched_Dirty"), Some(DecalKind::Marking), "dirty is not dirt");
        assert_eq!(classify("INI_Dirt_TireBend"), Some(DecalKind::Grime));
    }

    #[test]
    fn stretched_decals_put_the_texture_top_up() {
        // Rotation 0: up is north. The northern edge must be T = 1.
        let p = texture_polygon(&square(41.97, -87.9, 46.0), 25.0, 0.0, true).unwrap();
        let north = p.iter().filter(|q| q.1 > 41.97 + 1e-6).collect::<Vec<_>>();
        assert!(north.iter().all(|q| (q.3 - 1.0).abs() < 1e-6), "{p:?}");
        let east = p.iter().filter(|q| q.0 > -87.9 + 1e-6).collect::<Vec<_>>();
        assert!(east.iter().all(|q| (q.2 - 1.0).abs() < 1e-6), "stretched once across");
    }

    #[test]
    fn stretched_decals_follow_the_rotation() {
        // Rotation -90 degrees: up is east, so the eastern edge is T = 1.
        let p = texture_polygon(&square(41.97, -87.9, 20.0), 25.0, -std::f32::consts::FRAC_PI_2, true).unwrap();
        let east = p.iter().filter(|q| q.0 > -87.9 + 1e-6).collect::<Vec<_>>();
        assert!(east.iter().all(|q| (q.3 - 1.0).abs() < 1e-4), "{p:?}");
    }

    #[test]
    fn tiled_decals_repeat_and_put_the_top_down() {
        // A 40 m by 4 m bar running east, rotation 0 (up north), 4 m repeats.
        let dlat = 4.0 / 110_574.0;
        let dlon = 40.0 / (111_320.0 * 41.97f64.to_radians().cos());
        let bar = vec![
            LatLon::new(41.97, -87.9),
            LatLon::new(41.97, -87.9 + dlon),
            LatLon::new(41.97 + dlat, -87.9 + dlon),
            LatLon::new(41.97 + dlat, -87.9),
        ];
        let p = texture_polygon(&bar, 4.0, 0.0, false).unwrap();
        let smax = p.iter().map(|q| q.2).fold(f64::MIN, f64::max);
        assert!((smax - 10.0).abs() < 0.1, "ten repeats along the bar: {smax}");
        // The southern ("down") edge carries the texture's top, T = 1.
        let south = p.iter().filter(|q| q.1 < 41.97 + 1e-7).collect::<Vec<_>>();
        assert!(south.iter().all(|q| (q.3 - 1.0).abs() < 1e-6), "{p:?}");
    }

    #[test]
    fn clockwise_outlines_are_reversed() {
        let mut cw = square(41.97, -87.9, 10.0);
        cw.reverse();
        let p = texture_polygon(&cw, 5.0, 0.0, false).unwrap();
        let plane = Plane::new(LatLon::new(p[0].1, p[0].0));
        let xy: Vec<(f64, f64)> = p.iter().map(|q| plane.to_xy(LatLon::new(q.1, q.0))).collect();
        let area: f64 = (0..4).map(|i| xy[i].0 * xy[(i + 1) % 4].1 - xy[(i + 1) % 4].0 * xy[i].1).sum();
        assert!(area > 0.0);
    }

    #[test]
    fn only_square_quads_count_as_square() {
        assert!(is_square_quad(&square(41.97, -87.9, 3.0)), "a 3 m stencil");
        let mut closed = square(41.97, -87.9, 46.0);
        closed.push(closed[0]);
        assert!(is_square_quad(&closed), "a closed ring of a square");
        let dlat = 22.0 / 110_574.0;
        let dlon = 12.0 / (111_320.0 * 41.97f64.to_radians().cos());
        let strip = vec![
            LatLon::new(41.97, -87.9),
            LatLon::new(41.97, -87.9 + dlon),
            LatLon::new(41.97 + dlat, -87.9 + dlon),
            LatLon::new(41.97 + dlat, -87.9),
        ];
        assert!(!is_square_quad(&strip), "a 12 m by 22 m hatched strip tiles");
        let mut five = square(41.97, -87.9, 10.0);
        five.insert(2, LatLon::new(41.97005, -87.8999));
        assert!(!is_square_quad(&five));
    }

    #[test]
    fn stretched_flag_needs_a_square_and_tiled_grime_is_dropped() {
        let base = Apron {
            draw: false,
            decal_texture: Some("T.PNG.KTX2".into()),
            uv_scale: 15.0,
            priority: 1,
            flags: 0x80,
            ..Default::default()
        };
        let dlat = 22.0 / 110_574.0;
        let dlon = 12.0 / (111_320.0 * 41.97f64.to_radians().cos());
        let strip = vec![
            LatLon::new(41.97, -87.9),
            LatLon::new(41.97, -87.9 + dlon),
            LatLon::new(41.97 + dlat, -87.9 + dlon),
            LatLon::new(41.97 + dlat, -87.9),
        ];
        let hatched = Apron {
            material_name: Some("INI_Decal_Hatched_Thin".into()),
            vertices: strip.clone(),
            ..base.clone()
        };
        let stencil = Apron {
            material_name: Some("INI_Decal_Stop_Long".into()),
            vertices: square(41.97, -87.9, 3.0),
            ..base.clone()
        };
        let tiled_dirt = Apron {
            material_name: Some("INI_Dirt_01".into()),
            vertices: strip,
            flags: 0x03,
            uv_scale: 25.0,
            ..base.clone()
        };
        let patch_dirt = Apron {
            material_name: Some("INI_Dirt_02".into()),
            vertices: square(41.97, -87.9, 50.0),
            ..base
        };
        let d = airport_decals(&[hatched, stencil, tiled_dirt, patch_dirt]);
        assert_eq!(d.len(), 3, "tiled grime is dropped: {d:?}");
        assert!(d.iter().any(|x| x.kind == DecalKind::Grime && x.stretched), "a square dirt patch stays");
        let stripes = d.iter().find(|x| x.texture == "T.PNG.KTX2" && !x.stretched && x.kind == DecalKind::Marking);
        assert!(stripes.is_some(), "the hatched strip tiles despite flag 0x80");
        assert!(d.iter().any(|x| x.stretched && x.kind == DecalKind::Marking), "the square stencil stretches");
    }

    #[test]
    fn degenerate_outlines_give_nothing() {
        let line = vec![LatLon::new(41.97, -87.9), LatLon::new(41.971, -87.9)];
        assert!(texture_polygon(&line, 4.0, 0.0, false).is_none());
    }

    #[test]
    fn pol_files_use_x_plane_keywords() {
        let t = pol_text("../textures/HOLDSHORT.dds", false, DecalKind::Marking);
        assert!(t.starts_with("A\n850\nDRAPED_POLYGON\n"));
        assert!(t.contains("\nTEXTURE ../textures/HOLDSHORT.dds\n"));
        assert!(t.contains("LAYER_GROUP markings -1"));
        assert!(!t.contains("NO_ALPHA"), "decals need their alpha");
        assert!(pol_text("x.dds", true, DecalKind::Grime).contains("TEXTURE_NOWRAP x.dds"));
    }
}
