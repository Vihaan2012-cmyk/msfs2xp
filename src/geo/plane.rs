//! A local tangent plane in metres.
//!
//! Polygon unions, offsets and model placement all need planar maths. Working
//! directly in degrees would distort everything away from the equator, so each
//! airport gets its own plane anchored at its reference point. Over the few
//! kilometres an airport spans, the error against the ellipsoid is millimetres.

use super::vincenty::LatLon;

/// Metres per degree of latitude and longitude at a given latitude.
#[derive(Debug, Clone, Copy)]
pub struct Plane {
    origin: LatLon,
    m_per_deg_lat: f64,
    m_per_deg_lon: f64,
}

impl Plane {
    pub fn new(origin: LatLon) -> Self {
        let phi = origin.lat.to_radians();
        // Standard series for the WGS84 ellipsoid.
        let m_per_deg_lat =
            111_132.92 - 559.82 * (2.0 * phi).cos() + 1.175 * (4.0 * phi).cos() - 0.0023 * (6.0 * phi).cos();
        let m_per_deg_lon = 111_412.84 * phi.cos() - 93.5 * (3.0 * phi).cos() + 0.118 * (5.0 * phi).cos();
        // Guard the poles, where a degree of longitude collapses to nothing.
        let m_per_deg_lon = if m_per_deg_lon.abs() < 1.0 { 1.0 } else { m_per_deg_lon };
        Plane {
            origin,
            m_per_deg_lat,
            m_per_deg_lon,
        }
    }

    pub fn origin(&self) -> LatLon {
        self.origin
    }

    /// Position in metres east and north of the origin.
    pub fn to_xy(&self, p: LatLon) -> (f64, f64) {
        let mut dlon = p.lon - self.origin.lon;
        if dlon > 180.0 {
            dlon -= 360.0;
        } else if dlon < -180.0 {
            dlon += 360.0;
        }
        (
            dlon * self.m_per_deg_lon,
            (p.lat - self.origin.lat) * self.m_per_deg_lat,
        )
    }

    /// Inverse of [`Plane::to_xy`].
    pub fn to_latlon(&self, x: f64, y: f64) -> LatLon {
        let lat = self.origin.lat + y / self.m_per_deg_lat;
        let mut lon = self.origin.lon + x / self.m_per_deg_lon;
        if lon > 180.0 {
            lon -= 360.0;
        } else if lon < -180.0 {
            lon += 360.0;
        }
        LatLon { lat, lon }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::vincenty::{destination, inverse};

    #[test]
    fn round_trips_to_the_millimetre() {
        let plane = Plane::new(LatLon::new(25.2528, 55.3644));
        for &(dlat, dlon) in &[(0.0, 0.0), (0.01, 0.01), (-0.02, 0.03), (0.005, -0.04)] {
            let p = LatLon::new(25.2528 + dlat, 55.3644 + dlon);
            let (x, y) = plane.to_xy(p);
            let back = plane.to_latlon(x, y);
            assert!((back.lat - p.lat).abs() < 1e-9);
            assert!((back.lon - p.lon).abs() < 1e-9);
        }
    }

    #[test]
    fn distances_agree_with_the_ellipsoid() {
        // Across a typical airport the planar approximation must stay within
        // a few centimetres of the geodesic.
        for lat in [0.0, 25.25, 51.5, 68.9] {
            let origin = LatLon::new(lat, 10.0);
            let plane = Plane::new(origin);
            for (brg, dist) in [(0.0, 3000.0), (90.0, 3000.0), (45.0, 5000.0)] {
                let p = destination(origin, brg, dist);
                let (x, y) = plane.to_xy(p);
                let planar = (x * x + y * y).sqrt();
                let err = (planar - dist).abs();
                assert!(err < 2.0, "lat {lat} brg {brg}: planar {planar} vs {dist}");
            }
        }
    }

    #[test]
    fn bearing_is_preserved() {
        let origin = LatLon::new(25.2528, 55.3644);
        let plane = Plane::new(origin);
        let p = destination(origin, 120.0, 2000.0);
        let (x, y) = plane.to_xy(p);
        let planar_bearing = crate::geo::vincenty::normalize_deg(x.atan2(y).to_degrees());
        assert!((planar_bearing - 120.0).abs() < 0.5, "got {planar_bearing}");
        let (_, geo_bearing) = inverse(origin, p);
        assert!((planar_bearing - geo_bearing).abs() < 0.5);
    }

    #[test]
    fn works_at_the_antimeridian() {
        let plane = Plane::new(LatLon::new(-16.9, 179.98));
        let p = LatLon::new(-16.9, -179.98);
        let (x, _) = plane.to_xy(p);
        // Four hundredths of a degree east, not most of the way around the world.
        assert!(x > 0.0 && x < 6000.0, "x was {x}");
        assert!((plane.to_latlon(x, 0.0).lon - p.lon).abs() < 1e-9);
    }

    #[test]
    fn survives_the_pole() {
        let plane = Plane::new(LatLon::new(90.0, 0.0));
        let (x, y) = plane.to_xy(LatLon::new(89.99, 10.0));
        assert!(x.is_finite() && y.is_finite());
    }
}
