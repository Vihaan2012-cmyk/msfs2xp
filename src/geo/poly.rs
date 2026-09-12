//! Planar polygon helpers: quads, discs, polyline offsets and boolean union.
//!
//! Taxiway surfaces in MSFS are a network of overlapping rectangles, one per
//! segment. Emitting them as-is gives X-Plane thousands of overlapping polygons
//! that z-fight. Unioning them per surface type produces the single outline a
//! human would have drawn.

use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon};

/// A closed ring of planar points, metres, counter-clockwise.
pub type Ring = Vec<(f64, f64)>;

/// One polygon with an outer ring and any holes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shape {
    pub outer: Ring,
    pub holes: Vec<Ring>,
}

/// The four corners of a rectangle of `width` centred on the segment `a`-`b`.
pub fn segment_quad(a: (f64, f64), b: (f64, f64), width: f64) -> Ring {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 || !width.is_finite() || width <= 0.0 {
        return Vec::new();
    }
    let (nx, ny) = (-dy / len * width / 2.0, dx / len * width / 2.0);
    vec![
        (a.0 + nx, a.1 + ny),
        (b.0 + nx, b.1 + ny),
        (b.0 - nx, b.1 - ny),
        (a.0 - nx, a.1 - ny),
    ]
}

/// An `n`-sided approximation of a circle, used to round off taxiway junctions.
pub fn disc(centre: (f64, f64), radius: f64, n: usize) -> Ring {
    let n = n.max(3);
    if !radius.is_finite() || radius <= 0.0 {
        return Vec::new();
    }
    (0..n)
        .map(|i| {
            let t = std::f64::consts::TAU * i as f64 / n as f64;
            (centre.0 + radius * t.cos(), centre.1 + radius * t.sin())
        })
        .collect()
}

/// Offset a polyline sideways by `offset` metres (positive is to the left).
///
/// Corners are mitred, with the miter clamped so that a hairpin bend cannot
/// throw a vertex far away from the original line.
pub fn offset_polyline(points: &[(f64, f64)], offset: f64) -> Vec<(f64, f64)> {
    let pts = dedup(points);
    if pts.len() < 2 || !offset.is_finite() {
        return Vec::new();
    }
    let normal = |i: usize| {
        let (a, b) = (pts[i], pts[i + 1]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        (-dy / len, dx / len)
    };
    let miter_limit = offset.abs() * 4.0;
    let mut out = Vec::with_capacity(pts.len());
    for i in 0..pts.len() {
        let (nx, ny) = if i == 0 {
            normal(0)
        } else if i == pts.len() - 1 {
            normal(pts.len() - 2)
        } else {
            let (ax, ay) = normal(i - 1);
            let (bx, by) = normal(i);
            let (mx, my) = (ax + bx, ay + by);
            let m = (mx * mx + my * my).sqrt();
            if m < 1e-9 {
                normal(i) // a perfect reversal; fall back to one side
            } else {
                // Scale the mitre so the offset distance is preserved.
                let scale = (2.0 / (m * m)).min(4.0);
                (mx * scale, my * scale)
            }
        };
        let (px, py) = (pts[i].0 + nx * offset, pts[i].1 + ny * offset);
        let (dx, dy) = (px - pts[i].0, py - pts[i].1);
        let d = (dx * dx + dy * dy).sqrt();
        if d > miter_limit && d > 1e-9 {
            let k = miter_limit / d;
            out.push((pts[i].0 + dx * k, pts[i].1 + dy * k));
        } else {
            out.push((px, py));
        }
    }
    out
}

/// Drop consecutive duplicate points, which otherwise produce zero-length
/// segments and NaN normals.
pub fn dedup(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for &p in points {
        if !p.0.is_finite() || !p.1.is_finite() {
            continue;
        }
        match out.last() {
            Some(&q) if (q.0 - p.0).abs() < 1e-6 && (q.1 - p.1).abs() < 1e-6 => {}
            _ => out.push(p),
        }
    }
    out
}

/// Signed area; positive means counter-clockwise.
pub fn signed_area(ring: &[(f64, f64)]) -> f64 {
    if ring.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        sum += a.0 * b.1 - b.0 * a.1;
    }
    sum / 2.0
}

fn to_geo(ring: &Ring) -> Polygon<f64> {
    let coords: Vec<Coord<f64>> = ring.iter().map(|&(x, y)| Coord { x, y }).collect();
    Polygon::new(LineString::from(coords), vec![])
}

fn from_geo(mp: MultiPolygon<f64>) -> Vec<Shape> {
    mp.0.into_iter()
        .filter_map(|poly| {
            let (exterior, interiors) = poly.into_inner();
            let outer = ring_from_linestring(&exterior);
            if outer.len() < 3 {
                return None;
            }
            let holes = interiors
                .iter()
                .map(ring_from_linestring)
                .filter(|h| h.len() >= 3)
                .collect();
            Some(Shape { outer, holes })
        })
        .collect()
}

fn ring_from_linestring(ls: &LineString<f64>) -> Ring {
    let mut pts: Vec<(f64, f64)> = ls.coords().map(|c| (c.x, c.y)).collect();
    // geo closes rings by repeating the first point; apt.dat does not.
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    dedup(&pts)
}

/// Union a set of rings into as few polygons as possible.
///
/// Returns an error rather than panicking if the geometry library rejects the
/// input, so the caller can fall back to emitting the rings untouched.
pub fn union(rings: Vec<Ring>) -> Result<Vec<Shape>, String> {
    let usable: Vec<&Ring> = rings.iter().filter(|r| r.len() >= 3).collect();
    if usable.is_empty() {
        return Ok(Vec::new());
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut acc = MultiPolygon::new(vec![to_geo(usable[0])]);
        for ring in usable.iter().skip(1) {
            let other = MultiPolygon::new(vec![to_geo(ring)]);
            acc = acc.union(&other);
        }
        acc
    }));
    match result {
        Ok(mp) => Ok(from_geo(mp)),
        Err(_) => Err("polygon union failed on degenerate geometry".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn quad_has_the_requested_width_and_length() {
        let q = segment_quad((0.0, 0.0), (100.0, 0.0), 20.0);
        assert_eq!(q.len(), 4);
        assert!(approx(signed_area(&q).abs(), 2000.0, 1e-6));
        // Corners sit half a width either side of the centreline.
        assert!(q.iter().all(|p| approx(p.1.abs(), 10.0, 1e-9)));
    }

    #[test]
    fn degenerate_quads_are_empty() {
        assert!(segment_quad((5.0, 5.0), (5.0, 5.0), 20.0).is_empty());
        assert!(segment_quad((0.0, 0.0), (10.0, 0.0), 0.0).is_empty());
        assert!(segment_quad((0.0, 0.0), (10.0, 0.0), f64::NAN).is_empty());
    }

    #[test]
    fn disc_area_approaches_a_circle() {
        let d = disc((0.0, 0.0), 10.0, 64);
        assert!(approx(signed_area(&d).abs(), std::f64::consts::PI * 100.0, 1.0));
        assert!(disc((0.0, 0.0), -1.0, 16).is_empty());
    }

    #[test]
    fn offset_moves_a_straight_line_sideways() {
        let line = [(0.0, 0.0), (100.0, 0.0)];
        let left = offset_polyline(&line, 5.0);
        let right = offset_polyline(&line, -5.0);
        assert_eq!(left.len(), 2);
        assert!(left.iter().all(|p| approx(p.1, 5.0, 1e-9)));
        assert!(right.iter().all(|p| approx(p.1, -5.0, 1e-9)));
    }

    #[test]
    fn offset_mitres_a_corner() {
        let line = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)];
        let off = offset_polyline(&line, 10.0);
        assert_eq!(off.len(), 3);
        // The inside corner moves diagonally, staying near the offset distance.
        let corner = off[1];
        assert!(corner.0 < 100.0 && corner.1 > 0.0, "corner was {corner:?}");
        let dist = ((corner.0 - 100.0).powi(2) + corner.1.powi(2)).sqrt();
        assert!(dist < 40.0, "mitre ran away: {dist}");
    }

    #[test]
    fn offset_ignores_repeated_points() {
        let line = [(0.0, 0.0), (0.0, 0.0), (50.0, 0.0)];
        let off = offset_polyline(&line, 3.0);
        assert_eq!(off.len(), 2);
        assert!(off.iter().all(|p| p.1.is_finite()));
    }

    #[test]
    fn union_merges_two_overlapping_quads() {
        let a = segment_quad((0.0, 0.0), (100.0, 0.0), 20.0);
        let b = segment_quad((50.0, 0.0), (150.0, 0.0), 20.0);
        let shapes = union(vec![a, b]).unwrap();
        assert_eq!(shapes.len(), 1, "overlapping quads should merge");
        assert!(approx(signed_area(&shapes[0].outer).abs(), 3000.0, 1.0));
    }

    #[test]
    fn union_keeps_disjoint_quads_apart() {
        let a = segment_quad((0.0, 0.0), (10.0, 0.0), 5.0);
        let b = segment_quad((500.0, 500.0), (510.0, 500.0), 5.0);
        assert_eq!(union(vec![a, b]).unwrap().len(), 2);
    }

    #[test]
    fn union_produces_a_hole_for_a_ring_of_quads() {
        // Four segments around a square leave an untouched centre.
        let w = 10.0;
        let quads = vec![
            segment_quad((0.0, 0.0), (100.0, 0.0), w),
            segment_quad((100.0, 0.0), (100.0, 100.0), w),
            segment_quad((100.0, 100.0), (0.0, 100.0), w),
            segment_quad((0.0, 100.0), (0.0, 0.0), w),
        ];
        let shapes = union(quads).unwrap();
        assert_eq!(shapes.len(), 1);
        assert_eq!(shapes[0].holes.len(), 1, "the enclosed centre should be a hole");
    }

    #[test]
    fn union_of_nothing_is_nothing() {
        assert!(union(vec![]).unwrap().is_empty());
        assert!(union(vec![vec![(0.0, 0.0), (1.0, 1.0)]]).unwrap().is_empty());
    }

    #[test]
    fn dedup_drops_repeats_and_non_finite() {
        let pts = [(0.0, 0.0), (0.0, 0.0), (1.0, 1.0), (f64::NAN, 2.0), (1.0, 1.0)];
        assert_eq!(dedup(&pts), vec![(0.0, 0.0), (1.0, 1.0)]);
    }
}
