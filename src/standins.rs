//! Stand-ins from X-Plane's library for models a package places but does not
//! carry.
//!
//! MSFS 2024 streams its stock objects, so a package built for it names
//! thousands of models this PC never sees: trees, car-park lamps, barriers.
//! Their names are not on disk either, only their GUIDs, so each model is
//! judged by where it stands. Every placement of one model is looked at
//! together: how high above the ground, how far apart, whether the points
//! line up and whether they sit on pavement. Tight straight lines are
//! barriers, spaced rows and grids are lamps (car-park lamps on pavement,
//! street lights along roads) and scattered points off the pavement are
//! trees. Headings tell rows of trees from rows
//! of lamps: MSFS turns trees at random, while lamps and barriers line up
//! with their row. Anything raised off the ground (roof fixtures, bridge
//! railings) or unclear is left out rather than guessed.

use std::collections::HashMap;

use crate::bgl::guid::Guid;
use crate::bgl::records::scenery::RawPlacement;
use crate::geo::inverse;
use crate::model::{Apron, Surface};

/// What a missing model is taken to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Tree,
    CarParkLight,
    StreetLight,
    Barrier,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Tree => "trees",
            Kind::CarParkLight => "car-park lights",
            Kind::StreetLight => "street lights",
            Kind::Barrier => "barriers",
        }
    }
}

/// One X-Plane library object standing in for one MSFS placement.
#[derive(Debug, Clone, PartialEq)]
pub struct StandIn {
    pub lat: f64,
    pub lon: f64,
    pub heading: f64,
    pub path: &'static str,
    pub kind: Kind,
}

/// How one missing model was judged, for the conversion report.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Verdict {
    pub guid: String,
    pub placements: usize,
    /// The stand-in chosen, or none when the model was left out.
    pub kind: Option<&'static str>,
    pub height_m: f64,
    pub spacing_m: f64,
    pub linear: f64,
    pub on_pavement: f64,
    pub aligned: f64,
}

const TREES: [&str; 4] = [
    "lib/g10/forests/autogen_tree1.obj",
    "lib/g10/forests/autogen_tree2.obj",
    "lib/g10/forests/autogen_tree3.obj",
    "lib/g10/forests/autogen_tree4.obj",
];
const PALMS: [&str; 4] = [
    "lib/g10/forests/Date_palm_medium.obj",
    "lib/g10/forests/Date_palm_tall.obj",
    "lib/g10/forests/Mexican_palm_medium.obj",
    "lib/g10/forests/Mexican_palm_tall.obj",
];
const CAR_PARK_LIGHT: &str = "lib/g10/streetlights/ParkingLot.obj";
const STREET_LIGHT: &str = "lib/g10/streetlights/PrimaryLt1.obj";
/// Concrete barriers by length; their long side runs along the object's X.
const BARRIERS: [(f64, &str); 3] = [
    (1.5, "lib/airport/Common_Elements/Barriers/concrete/grey_1_5m.obj"),
    (3.0, "lib/airport/Common_Elements/Barriers/concrete/grey_3m.obj"),
    (6.0, "lib/airport/Common_Elements/Barriers/concrete/grey_6m.obj"),
];

/// Point-in-pavement test over the airport's apron outlines.
struct Pavement {
    rings: Vec<Vec<(f64, f64)>>,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

/// Grid cell size in degrees (about 100 m).
const CELL: f64 = 0.001;

fn cell(v: f64) -> i64 {
    (v / CELL).floor() as i64
}

impl Pavement {
    fn new(aprons: &[&Apron]) -> Self {
        let mut rings = Vec::new();
        let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        // Only drawn, hard surfaces: MSFS aprons also paint grass and sand.
        let hard = |s: &Surface| {
            matches!(
                s,
                Surface::Concrete
                    | Surface::Cement
                    | Surface::Asphalt
                    | Surface::Bituminous
                    | Surface::Tarmac
                    | Surface::Macadam
                    | Surface::OilTreated
                    | Surface::Brick
            )
        };
        for a in aprons.iter().filter(|a| a.vertices.len() >= 3 && a.draw && hard(&a.surface)) {
            let ring: Vec<(f64, f64)> = a.vertices.iter().map(|v| (v.lon, v.lat)).collect();
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for &(x, y) in &ring {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
            // Skip absurd outlines rather than fill half the map with cells.
            if (x1 - x0) > 0.2 || (y1 - y0) > 0.2 {
                continue;
            }
            let i = rings.len();
            rings.push(ring);
            for cx in cell(x0)..=cell(x1) {
                for cy in cell(y0)..=cell(y1) {
                    cells.entry((cx, cy)).or_default().push(i);
                }
            }
        }
        Pavement { rings, cells }
    }

    fn contains(&self, lat: f64, lon: f64) -> bool {
        let Some(list) = self.cells.get(&(cell(lon), cell(lat))) else {
            return false;
        };
        list.iter().any(|&i| {
            let ring = &self.rings[i];
            let mut inside = false;
            let mut j = ring.len() - 1;
            for k in 0..ring.len() {
                let ((xi, yi), (xj, yj)) = (ring[k], ring[j]);
                if (yi > lat) != (yj > lat) && lon < (xj - xi) * (lat - yi) / (yj - yi) + xi {
                    inside = !inside;
                }
                j = k;
            }
            inside
        })
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// How one model's placements are laid out.
struct Layout {
    /// Median distance to the nearest other placement, metres.
    spacing: f64,
    /// Share of placements whose two nearest neighbours lie on either side
    /// in a straight line.
    linear: f64,
    /// Share of placements on pavement.
    on_pavement: f64,
    /// Share of placements whose heading lines up (within 10 degrees) with
    /// the direction to their nearest neighbour, or square to it. About 0.2
    /// for random headings.
    aligned: f64,
    /// Bearing to each placement's nearest neighbour.
    along: Vec<f64>,
}

fn layout(ps: &[&RawPlacement], pavement: &Pavement) -> Layout {
    let lat0 = ps[0].lat;
    let k = lat0.to_radians().cos();
    let xy: Vec<(f64, f64)> = ps
        .iter()
        .map(|p| ((p.lon - ps[0].lon) * 111_320.0 * k, (p.lat - lat0) * 110_574.0))
        .collect();
    let mut spacing = Vec::with_capacity(xy.len());
    let mut straight = 0usize;
    let mut along = Vec::with_capacity(xy.len());
    for (i, a) in xy.iter().enumerate() {
        let (mut best, mut second) = ((f64::INFINITY, i), (f64::INFINITY, i));
        for (j, b) in xy.iter().enumerate() {
            if j == i {
                continue;
            }
            let d = (b.0 - a.0).hypot(b.1 - a.1);
            if d < best.0 {
                second = best;
                best = (d, j);
            } else if d < second.0 {
                second = (d, j);
            }
        }
        spacing.push(best.0);
        let (b, c) = (xy[best.1], xy[second.1]);
        let (u, v) = ((b.0 - a.0, b.1 - a.1), (c.0 - a.0, c.1 - a.1));
        let norm = u.0.hypot(u.1) * v.0.hypot(v.1);
        // Neighbours on opposite sides, within 30 degrees of a straight line.
        if norm > 0.0 && (u.0 * v.0 + u.1 * v.1) / norm < -0.866 {
            straight += 1;
        }
        along.push(inverse(
            crate::geo::LatLon::new(ps[i].lat, ps[i].lon),
            crate::geo::LatLon::new(ps[best.1].lat, ps[best.1].lon),
        )
        .1);
    }
    let aligned = ps
        .iter()
        .zip(&along)
        .filter(|(p, b)| {
            let d = (p.heading as f64 - **b).rem_euclid(90.0);
            d.min(90.0 - d) <= 10.0
        })
        .count();
    let on = ps.iter().filter(|p| pavement.contains(p.lat, p.lon)).count();
    Layout {
        spacing: median(spacing),
        linear: straight as f64 / ps.len() as f64,
        on_pavement: on as f64 / ps.len() as f64,
        aligned: aligned as f64 / ps.len() as f64,
        along,
    }
}

/// What a model is, from the layout of its placements.
fn classify(heights: &[f64], l: &Layout) -> Option<Kind> {
    if heights.len() < 8 || median(heights.to_vec()).abs() > 1.0 {
        return None;
    }
    if l.spacing <= 7.0 && l.linear >= 0.6 {
        Some(Kind::Barrier)
    } else if l.aligned <= 0.35 && l.on_pavement <= 0.5 && l.spacing >= 4.0 {
        // Random headings, off the pavement: trees.
        Some(Kind::Tree)
    } else if l.aligned >= 0.45 && (12.0..=70.0).contains(&l.spacing) {
        // Headings that follow the row: lamps, in a car park or along a road.
        Some(if l.on_pavement >= 0.6 {
            Kind::CarParkLight
        } else {
            Kind::StreetLight
        })
    } else {
        None
    }
}

/// A stable pick from a list, varied by position.
fn pick<'a>(list: &[&'a str], lat: f64, lon: f64) -> &'a str {
    let h = ((lat * 1e6).round() as i64).wrapping_mul(73_856_093) ^ ((lon * 1e6).round() as i64).wrapping_mul(19_349_663);
    list[(h.unsigned_abs() % list.len() as u64) as usize]
}

/// Stand-ins for placements whose models are not on this PC, and how each
/// model was judged (most placed first). `height` gives a placement's height
/// above the ground; `hot` selects palms over trees.
pub fn stand_ins(
    missing: &[&RawPlacement],
    aprons: &[&Apron],
    hot: bool,
    height: &dyn Fn(&RawPlacement) -> f64,
) -> (Vec<StandIn>, Vec<Verdict>) {
    let pavement = Pavement::new(aprons);
    let mut by: HashMap<Guid, Vec<&RawPlacement>> = HashMap::new();
    for p in missing {
        by.entry(p.guid).or_default().push(p);
    }
    let mut groups: Vec<(Guid, Vec<&RawPlacement>)> = by.into_iter().collect();
    groups.sort_by_key(|(g, _)| g.to_string());
    let mut out = Vec::new();
    let mut verdicts = Vec::new();
    for (guid, ps) in groups {
        let heights: Vec<f64> = ps.iter().map(|p| height(p)).collect();
        if ps.len() < 8 {
            continue;
        }
        let l = layout(&ps, &pavement);
        let judged = classify(&heights, &l);
        let round = |v: f64| (v * 100.0).round() / 100.0;
        verdicts.push(Verdict {
            guid: guid.to_string(),
            placements: ps.len(),
            kind: judged.map(Kind::label),
            height_m: round(median(heights)),
            spacing_m: round(l.spacing),
            linear: round(l.linear),
            on_pavement: round(l.on_pavement),
            aligned: round(l.aligned),
        });
        let Some(kind) = judged else { continue };
        let barrier = BARRIERS
            .iter()
            .min_by(|a, b| (a.0 - l.spacing).abs().total_cmp(&(b.0 - l.spacing).abs()))
            .map_or(BARRIERS[1].1, |b| b.1);
        for (i, p) in ps.iter().enumerate() {
            let (path, heading) = match kind {
                Kind::Tree => (pick(if hot { &PALMS } else { &TREES }, p.lat, p.lon), p.heading as f64),
                Kind::CarParkLight => (CAR_PARK_LIGHT, p.heading as f64),
                Kind::StreetLight => (STREET_LIGHT, p.heading as f64),
                // The barrier's length runs along its X axis, which points
                // 90 degrees clockwise of its heading.
                Kind::Barrier => (barrier, (l.along[i] - 90.0).rem_euclid(360.0)),
            };
            out.push(StandIn {
                lat: p.lat,
                lon: p.lon,
                heading,
                path,
                kind,
            });
        }
    }
    verdicts.sort_by_key(|v| std::cmp::Reverse(v.placements));
    (out, verdicts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::LatLon;

    fn at(guid: u8, lat: f64, lon: f64, alt: f64) -> RawPlacement {
        let mut g = [0u8; 16];
        g[0] = guid;
        RawPlacement {
            lat,
            lon,
            alt_m: alt,
            agl: true,
            pitch: 0.0,
            bank: 0.0,
            heading: 0.0,
            scale: 1.0,
            guid: Guid(g),
        }
    }

    /// Metres to degrees near 41.97 N.
    fn east(m: f64) -> f64 {
        m / (111_320.0 * 41.97f64.to_radians().cos())
    }
    fn north(m: f64) -> f64 {
        m / 110_574.0
    }

    fn run(ps: &[RawPlacement], aprons: &[Apron]) -> Vec<StandIn> {
        let refs: Vec<&RawPlacement> = ps.iter().collect();
        let aprons: Vec<&Apron> = aprons.iter().collect();
        stand_ins(&refs, &aprons, false, &|p: &RawPlacement| p.alt_m).0
    }

    fn square(lat: f64, lon: f64, metres: f64) -> Apron {
        Apron {
            vertices: vec![
                LatLon::new(lat, lon),
                LatLon::new(lat + north(metres), lon),
                LatLon::new(lat + north(metres), lon + east(metres)),
                LatLon::new(lat, lon + east(metres)),
            ],
            draw: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_tight_straight_line_is_a_barrier_along_the_line() {
        let ps: Vec<_> = (0..20).map(|i| at(1, 41.97, -87.9 + east(3.0 * i as f64), 0.0)).collect();
        let out = run(&ps, &[]);
        assert_eq!(out.len(), 20);
        assert!(out.iter().all(|s| s.kind == Kind::Barrier));
        assert!(out[0].path.ends_with("grey_3m.obj"));
        // The line runs east-west, so the barrier's X axis must too.
        assert!(out.iter().all(|s| s.heading.rem_euclid(180.0).min(180.0 - s.heading.rem_euclid(180.0)) < 1.0));
    }

    #[test]
    fn a_grid_on_pavement_is_car_park_lighting() {
        let mut ps = Vec::new();
        for i in 0..5 {
            for j in 0..5 {
                ps.push(at(2, 41.97 + north(10.0 + 25.0 * i as f64), -87.9 + east(10.0 + 25.0 * j as f64), 0.0));
            }
        }
        let out = run(&ps, &[square(41.97, -87.9, 150.0)]);
        assert_eq!(out.len(), 25);
        assert!(out.iter().all(|s| s.kind == Kind::CarParkLight));
    }

    #[test]
    fn scattered_points_off_the_pavement_are_trees() {
        let ps: Vec<_> = (0..30)
            .map(|i| {
                let (a, r) = (i as f64 * 2.4, 20.0 + 9.0 * i as f64);
                let mut p = at(3, 41.97 + north(r * a.sin()), -87.9 + east(r * a.cos()), 0.0);
                p.heading = (i * 53 % 360) as f32; // MSFS turns trees at random
                p
            })
            .collect();
        let out = run(&ps, &[]);
        assert_eq!(out.len(), 30);
        assert!(out.iter().all(|s| s.kind == Kind::Tree && TREES.contains(&s.path)));
        assert!(out.iter().map(|s| s.path).collect::<std::collections::HashSet<_>>().len() > 1, "trees vary");
    }

    #[test]
    fn an_aligned_row_along_a_road_is_street_lighting() {
        // 30 m apart along an east-west road, every lamp turned to face it.
        let ps: Vec<_> = (0..20)
            .map(|i| {
                let mut p = at(6, 41.97, -87.9 + east(30.0 * i as f64), 0.0);
                p.heading = 180.0;
                p
            })
            .collect();
        let out = run(&ps, &[]);
        assert_eq!(out.len(), 20);
        assert!(out.iter().all(|s| s.kind == Kind::StreetLight), "{:?}", out[0].kind);
    }

    #[test]
    fn raised_or_rare_models_are_left_out() {
        let raised: Vec<_> = (0..20).map(|i| at(4, 41.97, -87.9 + east(3.0 * i as f64), 9.8)).collect();
        assert!(run(&raised, &[]).is_empty(), "roof or bridge fixtures are not guessed");
        let rare: Vec<_> = (0..5).map(|i| at(5, 41.97, -87.9 + east(30.0 * i as f64), 0.0)).collect();
        assert!(run(&rare, &[]).is_empty(), "too few placements to judge");
    }
}
