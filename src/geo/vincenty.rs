//! WGS84 geodesics.
//!
//! Runway ends are derived from a centre point, a true heading and a length, so
//! the accuracy of this module sets the accuracy of the whole conversion. At a
//! 4 km runway a spherical approximation is off by several metres, which is
//! visible when an aircraft lines up, so the full Vincenty solution is used.

/// A WGS84 position in degrees.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

impl LatLon {
    pub fn new(lat: f64, lon: f64) -> Self {
        LatLon { lat, lon }
    }

    pub fn is_valid(&self) -> bool {
        self.lat.is_finite()
            && self.lon.is_finite()
            && (-90.0..=90.0).contains(&self.lat)
            && (-180.0..=180.0).contains(&self.lon)
    }
}

const A: f64 = 6_378_137.0;
const F: f64 = 1.0 / 298.257_223_563;
const B: f64 = A * (1.0 - F);

/// Normalise a bearing or heading into `[0, 360)`.
pub fn normalize_deg(d: f64) -> f64 {
    let mut d = d % 360.0;
    if d < 0.0 {
        d += 360.0;
    }
    d
}

/// The point reached from `start` on `bearing_deg` after `dist_m` (Vincenty direct).
pub fn destination(start: LatLon, bearing_deg: f64, dist_m: f64) -> LatLon {
    if dist_m == 0.0 || !dist_m.is_finite() {
        return start;
    }
    let alpha1 = bearing_deg.to_radians();
    let (sin_alpha1, cos_alpha1) = alpha1.sin_cos();
    let phi1 = start.lat.to_radians();
    let tan_u1 = (1.0 - F) * phi1.tan();
    let cos_u1 = 1.0 / (1.0 + tan_u1 * tan_u1).sqrt();
    let sin_u1 = tan_u1 * cos_u1;

    let sigma1 = tan_u1.atan2(cos_alpha1);
    let sin_alpha = cos_u1 * sin_alpha1;
    let cos_sq_alpha = 1.0 - sin_alpha * sin_alpha;
    let u_sq = cos_sq_alpha * (A * A - B * B) / (B * B);
    let cap_a = 1.0 + u_sq / 16384.0 * (4096.0 + u_sq * (-768.0 + u_sq * (320.0 - 175.0 * u_sq)));
    let cap_b = u_sq / 1024.0 * (256.0 + u_sq * (-128.0 + u_sq * (74.0 - 47.0 * u_sq)));

    let mut sigma = dist_m / (B * cap_a);
    let mut cos2_sigma_m;
    let (mut sin_sigma, mut cos_sigma);
    let mut iterations = 0;
    loop {
        cos2_sigma_m = (2.0 * sigma1 + sigma).cos();
        sin_sigma = sigma.sin();
        cos_sigma = sigma.cos();
        let delta_sigma = cap_b
            * sin_sigma
            * (cos2_sigma_m
                + cap_b / 4.0
                    * (cos_sigma * (-1.0 + 2.0 * cos2_sigma_m * cos2_sigma_m)
                        - cap_b / 6.0
                            * cos2_sigma_m
                            * (-3.0 + 4.0 * sin_sigma * sin_sigma)
                            * (-3.0 + 4.0 * cos2_sigma_m * cos2_sigma_m)));
        let sigma_new = dist_m / (B * cap_a) + delta_sigma;
        if (sigma_new - sigma).abs() < 1e-12 || iterations > 100 {
            sigma = sigma_new;
            break;
        }
        sigma = sigma_new;
        iterations += 1;
    }

    let tmp = sin_u1 * sin_sigma - cos_u1 * cos_sigma * cos_alpha1;
    let phi2 = (sin_u1 * cos_sigma + cos_u1 * sin_sigma * cos_alpha1)
        .atan2((1.0 - F) * (sin_alpha * sin_alpha + tmp * tmp).sqrt());
    let lambda = (sin_sigma * sin_alpha1).atan2(cos_u1 * cos_sigma - sin_u1 * sin_sigma * cos_alpha1);
    let cap_c = F / 16.0 * cos_sq_alpha * (4.0 + F * (4.0 - 3.0 * cos_sq_alpha));
    let l = lambda
        - (1.0 - cap_c)
            * F
            * sin_alpha
            * (sigma
                + cap_c * sin_sigma * (cos2_sigma_m + cap_c * cos_sigma * (-1.0 + 2.0 * cos2_sigma_m * cos2_sigma_m)));

    let lat = phi2.to_degrees();
    let mut lon = start.lon + l.to_degrees();
    if lon > 180.0 {
        lon -= 360.0;
    } else if lon < -180.0 {
        lon += 360.0;
    }
    LatLon { lat, lon }
}

/// Distance in metres and initial bearing in degrees from `a` to `b`.
pub fn inverse(a: LatLon, b: LatLon) -> (f64, f64) {
    let phi1 = a.lat.to_radians();
    let phi2 = b.lat.to_radians();
    let l = (b.lon - a.lon).to_radians();

    let tan_u1 = (1.0 - F) * phi1.tan();
    let cos_u1 = 1.0 / (1.0 + tan_u1 * tan_u1).sqrt();
    let sin_u1 = tan_u1 * cos_u1;
    let tan_u2 = (1.0 - F) * phi2.tan();
    let cos_u2 = 1.0 / (1.0 + tan_u2 * tan_u2).sqrt();
    let sin_u2 = tan_u2 * cos_u2;

    let mut lambda = l;
    let (mut sin_sigma, mut cos_sigma, mut sigma) = (0.0, 0.0, 0.0);
    let (mut cos_sq_alpha, mut cos2_sigma_m) = (0.0, 0.0);
    for _ in 0..200 {
        let (sin_lambda, cos_lambda) = lambda.sin_cos();
        let t1 = cos_u2 * sin_lambda;
        let t2 = cos_u1 * sin_u2 - sin_u1 * cos_u2 * cos_lambda;
        sin_sigma = (t1 * t1 + t2 * t2).sqrt();
        if sin_sigma == 0.0 {
            return (0.0, 0.0); // coincident points
        }
        cos_sigma = sin_u1 * sin_u2 + cos_u1 * cos_u2 * cos_lambda;
        sigma = sin_sigma.atan2(cos_sigma);
        let sin_alpha = cos_u1 * cos_u2 * sin_lambda / sin_sigma;
        cos_sq_alpha = 1.0 - sin_alpha * sin_alpha;
        cos2_sigma_m = if cos_sq_alpha != 0.0 {
            cos_sigma - 2.0 * sin_u1 * sin_u2 / cos_sq_alpha
        } else {
            0.0 // equatorial line
        };
        let c = F / 16.0 * cos_sq_alpha * (4.0 + F * (4.0 - 3.0 * cos_sq_alpha));
        let lambda_prev = lambda;
        lambda = l
            + (1.0 - c)
                * F
                * sin_alpha
                * (sigma + c * sin_sigma * (cos2_sigma_m + c * cos_sigma * (-1.0 + 2.0 * cos2_sigma_m * cos2_sigma_m)));
        if (lambda - lambda_prev).abs() < 1e-12 {
            break;
        }
    }

    let u_sq = cos_sq_alpha * (A * A - B * B) / (B * B);
    let cap_a = 1.0 + u_sq / 16384.0 * (4096.0 + u_sq * (-768.0 + u_sq * (320.0 - 175.0 * u_sq)));
    let cap_b = u_sq / 1024.0 * (256.0 + u_sq * (-128.0 + u_sq * (74.0 - 47.0 * u_sq)));
    let delta_sigma = cap_b
        * sin_sigma
        * (cos2_sigma_m
            + cap_b / 4.0
                * (cos_sigma * (-1.0 + 2.0 * cos2_sigma_m * cos2_sigma_m)
                    - cap_b / 6.0
                        * cos2_sigma_m
                        * (-3.0 + 4.0 * sin_sigma * sin_sigma)
                        * (-3.0 + 4.0 * cos2_sigma_m * cos2_sigma_m)));
    let distance = B * cap_a * (sigma - delta_sigma);

    let (sin_lambda, cos_lambda) = lambda.sin_cos();
    let bearing = (cos_u2 * sin_lambda).atan2(cos_u1 * sin_u2 - sin_u1 * cos_u2 * cos_lambda);
    (distance, normalize_deg(bearing.to_degrees()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_long_distance() {
        // JFK to LAX: 2 151 nautical miles, about 3 983 km.
        let jfk = LatLon::new(40.6398, -73.7789);
        let lax = LatLon::new(33.9425, -118.4081);
        let (d, brg) = inverse(jfk, lax);
        assert!((d - 3_983_000.0).abs() < 2_000.0, "distance was {d}");
        assert!((brg - 273.7).abs() < 1.5, "bearing was {brg}");

        // London Heathrow to Sydney, a near-antipodal case where the simple
        // spherical formula loses accuracy and Vincenty must still converge.
        let lhr = LatLon::new(51.4700, -0.4543);
        let syd = LatLon::new(-33.9399, 151.1753);
        let (d, _) = inverse(lhr, syd);
        assert!((d - 17_015_000.0).abs() < 15_000.0, "distance was {d}");
    }

    #[test]
    fn destination_matches_inverse() {
        let start = LatLon::new(25.2528, 55.3644);
        for &(brg, dist) in &[(0.0, 1000.0), (90.0, 4000.0), (187.3, 250.0), (359.9, 12_000.0)] {
            let end = destination(start, brg, dist);
            let (d, b) = inverse(start, end);
            assert!((d - dist).abs() < 0.01, "distance {d} vs {dist}");
            assert!((normalize_deg(b - brg)).min(360.0 - normalize_deg(b - brg)) < 0.001);
        }
    }

    #[test]
    fn runway_ends_are_symmetric() {
        // A 4 km runway: both ends must be 2 km from the centre and 4 km apart.
        let centre = LatLon::new(25.2528, 55.3644);
        let hdg = 120.0;
        let a = destination(centre, hdg + 180.0, 2000.0);
        let b = destination(centre, hdg, 2000.0);
        let (len, _) = inverse(a, b);
        assert!((len - 4000.0).abs() < 0.05, "length {len}");
        assert!((inverse(centre, a).0 - 2000.0).abs() < 0.01);
        assert!((inverse(centre, b).0 - 2000.0).abs() < 0.01);
    }

    #[test]
    fn crosses_the_antimeridian() {
        let start = LatLon::new(-16.9, 179.98);
        let end = destination(start, 90.0, 5000.0);
        assert!(end.lon < 0.0, "should wrap to negative longitude, got {}", end.lon);
        assert!(end.is_valid());
        let (d, _) = inverse(start, end);
        assert!((d - 5000.0).abs() < 0.1);
    }

    #[test]
    fn handles_poles_and_zero_distance() {
        let p = LatLon::new(89.999, 15.0);
        assert!(destination(p, 45.0, 100.0).is_valid());
        let s = LatLon::new(10.0, 20.0);
        assert_eq!(destination(s, 45.0, 0.0), s);
        assert_eq!(inverse(s, s).0, 0.0);
    }

    #[test]
    fn normalises_degrees() {
        assert_eq!(normalize_deg(-10.0), 350.0);
        assert_eq!(normalize_deg(370.0), 10.0);
        assert_eq!(normalize_deg(0.0), 0.0);
    }
}
