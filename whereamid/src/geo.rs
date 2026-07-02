//! Shared geographic helpers. Currently exposes `haversine_m`, the
//! great-circle distance in meters between two (lat, lon) points.
//!
//! Previously each consumer (trilaterate.rs, history.rs, the proptest
//! suite) had its own private copy. task-0081 consolidated them here so
//! a single implementation is the source of truth.

/// Great-circle distance in meters between two (lat, lon) points,
/// computed via the haversine formula on a sphere of mean Earth radius
/// 6,371 km. Inputs are degrees.
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    R * c
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Copenhagen <-> Malmö is ~28 km. Anchor the formula against a known
    /// real-world distance to a few-percent tolerance.
    #[test]
    fn haversine_copenhagen_malmoe() {
        let dist = haversine_m(55.6761, 12.5683, 55.6050, 13.0038);
        assert!(
            (dist - 28_000.0).abs() < 2000.0,
            "expected ~28 km, got {dist} m"
        );
    }

    /// Distance from a point to itself is 0.
    #[test]
    fn haversine_zero_at_identity() {
        assert!(haversine_m(55.0, 12.0, 55.0, 12.0) < 1e-6);
    }
}

// task-0082: Kani harness for haversine_m.
//
// Honest caveat: Kani's f64 reasoning is limited. sin/cos/asin/sqrt are
// modelled approximately and exhaustive search across 4×64 bits of
// floating-point input does not terminate in practical time. This
// harness is written so the panic-freedom property is *expressible*; if
// it is too slow to prove in CI, comment it out — the proptest in
// `proptests.rs::trilaterate_within_cluster_diameter` exercises the
// same surface stochastically.
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    #[kani::proof]
    fn haversine_m_no_panic_on_finite_input() {
        let lat1: f64 = kani::any();
        let lon1: f64 = kani::any();
        let lat2: f64 = kani::any();
        let lon2: f64 = kani::any();
        kani::assume(lat1.is_finite() && lon1.is_finite());
        kani::assume(lat2.is_finite() && lon2.is_finite());
        let _ = haversine_m(lat1, lon1, lat2, lon2);
    }
}
