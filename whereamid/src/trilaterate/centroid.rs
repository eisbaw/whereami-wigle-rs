//! Unit-sphere vector helpers and the independent visible-AP consensus score.
//!
//! These are the low-level primitives the rest of trilateration builds on:
//! the (lat, lon) <-> unit-vector maps that keep the centroid math off the
//! antimeridian/pole singularities, and a standalone plausibility check used
//! by the server to sanity-test a candidate fix against nearby cached APs.

/// Radius used by the independent visible-AP consensus plausibility check.
const CONSENSUS_INLIER_RADIUS_M: f64 = 200.0;
const MIN_CONSENSUS_N: usize = 3;

/// Fraction of visible, fixed cached AP coordinates within 200 meters of a
/// candidate. The 200-meter boundary is inclusive. Fewer than three cached
/// coordinates cannot establish consensus and therefore return `None`.
pub fn ap_consensus_score(visible_cached: &[(f64, f64)], candidate: (f64, f64)) -> Option<f64> {
    if visible_cached.len() < MIN_CONSENSUS_N {
        return None;
    }
    let inliers = visible_cached
        .iter()
        .filter(|&&(lat, lon)| {
            crate::geo::haversine_m(candidate.0, candidate.1, lat, lon) <= CONSENSUS_INLIER_RADIUS_M
        })
        .count();
    Some(inliers as f64 / visible_cached.len() as f64)
}

/// Map (lat, lon) in degrees to a 3D unit vector on the sphere.
pub(crate) fn to_unit_vec(lat_deg: f64, lon_deg: f64) -> (f64, f64, f64) {
    let lat = lat_deg.to_radians();
    let lon = lon_deg.to_radians();
    let cl = lat.cos();
    (cl * lon.cos(), cl * lon.sin(), lat.sin())
}

/// Map a (not necessarily unit) 3-vector back to (lat, lon) in degrees.
/// The vector is normalized internally.
pub(crate) fn from_unit_vec(x: f64, y: f64, z: f64) -> (f64, f64) {
    let mag = (x * x + y * y + z * z).sqrt();
    // Caller is expected to guard against mag ~ 0 (antipodal cancellation);
    // we still divide to produce *something* finite for the fallback paths.
    let n = if mag > 0.0 { mag } else { 1.0 };
    let lat = (z / n).clamp(-1.0, 1.0).asin().to_degrees();
    let lon = y.atan2(x).to_degrees();
    (lat, lon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consensus_requires_three_visible_cached_aps() {
        assert_eq!(ap_consensus_score(&[], (55.0, 12.0)), None);
        assert_eq!(
            ap_consensus_score(&[(55.0, 12.0), (55.0001, 12.0001)], (55.0, 12.0)),
            None
        );
    }

    #[test]
    fn consensus_counts_fraction_within_radius() {
        let visible = [(55.0, 12.0), (55.0005, 12.0), (56.0, 13.0), (57.0, 14.0)];
        assert_eq!(ap_consensus_score(&visible, (55.0, 12.0)), Some(0.5));
    }

    #[test]
    fn consensus_two_hundred_meter_boundary_is_inclusive() {
        // Moving north by distance/R gives an exact 200m great-circle arc on
        // the same meridian under the shared spherical Earth model.
        let delta_lat = (200.0_f64 / 6_371_000.0).to_degrees();
        let visible = [(delta_lat, 0.0), (0.0, 0.0), (0.0, 0.0)];
        let score = ap_consensus_score(&visible, (0.0, 0.0)).unwrap();
        assert_eq!(score, 1.0);
    }

    #[test]
    fn consensus_just_outside_two_hundred_meters_is_excluded() {
        let delta_lat = (200.001_f64 / 6_371_000.0).to_degrees();
        let visible = [(delta_lat, 0.0), (0.0, 0.0), (0.0, 0.0)];
        let score = ap_consensus_score(&visible, (0.0, 0.0)).unwrap();
        assert_eq!(score, 2.0 / 3.0);
    }
}
