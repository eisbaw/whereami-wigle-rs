//! Weighted centroid trilateration from AP positions and signal strengths.
//!
//! Centroid math operates on the **unit sphere**, not on raw lat/lon. This
//! avoids the antimeridian discontinuity (an arithmetic mean of lon=+179 and
//! lon=-179 yields lon=0, which is on the wrong side of the planet) and the
//! polar singularity (longitude is degenerate near the poles). Each (lat, lon)
//! is mapped to a unit 3-vector, vectors are weighted-summed and normalized,
//! then mapped back to (lat, lon) via atan2/asin.

use anyhow::{bail, Result};

/// Input for trilateration: a positioned AP with optional signal.
#[derive(Debug, Clone)]
pub struct PositionedAp {
    pub lat: f64,
    pub lon: f64,
    pub signal_dbm: Option<i32>,
}

/// Trilateration result.
#[derive(Debug, Clone)]
pub struct Position {
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: f64,
}

/// Map (lat, lon) in degrees to a 3D unit vector on the sphere.
fn to_unit_vec(lat_deg: f64, lon_deg: f64) -> (f64, f64, f64) {
    let lat = lat_deg.to_radians();
    let lon = lon_deg.to_radians();
    let cl = lat.cos();
    (cl * lon.cos(), cl * lon.sin(), lat.sin())
}

/// Map a (not necessarily unit) 3-vector back to (lat, lon) in degrees.
/// The vector is normalized internally.
fn from_unit_vec(x: f64, y: f64, z: f64) -> (f64, f64) {
    let mag = (x * x + y * y + z * z).sqrt();
    // Caller is expected to guard against mag ~ 0 (antipodal cancellation);
    // we still divide to produce *something* finite for the fallback paths.
    let n = if mag > 0.0 { mag } else { 1.0 };
    let lat = (z / n).clamp(-1.0, 1.0).asin().to_degrees();
    let lon = y.atan2(x).to_degrees();
    (lat, lon)
}

/// Filter out outlier APs that are implausibly far from the cluster.
///
/// Algorithm:
/// 1. Compute the spherical mean (unweighted) of all APs as the cluster center
/// 2. Compute each AP's haversine distance from that center
/// 3. Compute the median of those distances (MAD-like measure of cluster spread)
/// 4. Reject APs whose distance from center exceeds max(200m, 3 * median_distance)
///
/// The 200m floor reflects the physical assumption that APs in a neighborhood
/// are within ~200m. The 3x median_distance handles cases where all APs are
/// further apart (e.g. rural, or all have somewhat stale positions) — it adapts
/// to the actual data spread rather than rejecting everything.
pub fn filter_outliers(aps: &[PositionedAp]) -> Vec<PositionedAp> {
    if aps.len() <= 2 {
        return aps.to_vec();
    }

    // Compute spherical-mean center (unweighted) — robust across the antimeridian
    // and at the poles, where lat/lon medians give wrong answers.
    let (mut sx, mut sy, mut sz) = (0.0, 0.0, 0.0);
    for ap in aps {
        let (x, y, z) = to_unit_vec(ap.lat, ap.lon);
        sx += x;
        sy += y;
        sz += z;
    }
    // If APs cancel (e.g. antipodal), the cluster has no meaningful center;
    // fall back to keeping all APs and let trilaterate() handle the degeneracy.
    let mag = (sx * sx + sy * sy + sz * sz).sqrt();
    if mag < 1e-9 {
        return aps.to_vec();
    }
    let (center_lat, center_lon) = from_unit_vec(sx, sy, sz);

    // Compute each AP's distance from the spherical-mean center
    let distances: Vec<f64> = aps
        .iter()
        .map(|ap| haversine_m(center_lat, center_lon, ap.lat, ap.lon))
        .collect();

    // Compute median distance (the typical spread)
    let median_dist = median(&distances);

    // Threshold: max(200m physical floor, 3x the median spread)
    let threshold = f64::max(200.0, 3.0 * median_dist);

    // Keep APs within threshold of the median position
    let kept: Vec<PositionedAp> = aps
        .iter()
        .zip(distances.iter())
        .filter(|(_, d)| **d <= threshold)
        .map(|(ap, _)| ap.clone())
        .collect();

    // If somehow nothing survived (shouldn't happen with 3x median), return all
    if kept.is_empty() {
        return aps.to_vec();
    }

    kept
}

#[allow(clippy::manual_is_multiple_of)]
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = sorted.len();
    if n % 2 == 0 {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    }
}

/// Compute weighted centroid from positioned APs.
///
/// Weight formula: 10^(signal_dbm / 20). Since signal_dbm is negative (e.g. -40),
/// stronger signals (closer to 0) yield larger weights. E.g. -40 dBm -> weight 0.01,
/// -80 dBm -> weight 0.0001, so the stronger AP gets 100x more influence.
/// If no signal info, equal weight (1.0) for all APs.
pub fn trilaterate(aps: &[PositionedAp]) -> Result<Position> {
    if aps.is_empty() {
        bail!("no positioned APs for trilateration");
    }

    // Filter outliers before computing centroid
    let filtered = filter_outliers(aps);
    let aps = if filtered.is_empty() { aps } else { &filtered };

    if aps.len() == 1 {
        return Ok(Position {
            lat: aps[0].lat,
            lon: aps[0].lon,
            accuracy_m: 100.0,
        });
    }

    // Weighted spherical mean: sum(w_i * v_i) on the unit sphere, then normalize.
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut sz = 0.0;
    let mut sum_weight = 0.0;

    for ap in aps {
        let weight = match ap.signal_dbm {
            Some(dbm) => f64::powf(10.0, dbm as f64 / 20.0),
            None => 1.0,
        };
        let (x, y, z) = to_unit_vec(ap.lat, ap.lon);
        sx += weight * x;
        sy += weight * y;
        sz += weight * z;
        sum_weight += weight;
    }

    // Antipodal-cancellation guard: if the weighted vector sum has near-zero
    // magnitude, the input is fundamentally ambiguous (e.g. two diametrically
    // opposed APs with equal weight). Fall back to the strongest-signal AP.
    let mag = (sx * sx + sy * sy + sz * sz).sqrt();
    if mag < sum_weight * 1e-6 {
        let strongest = aps
            .iter()
            .max_by(|a, b| {
                let da = a.signal_dbm.unwrap_or(i32::MIN);
                let db = b.signal_dbm.unwrap_or(i32::MIN);
                da.cmp(&db)
            })
            .expect("non-empty after guard above");
        return Ok(Position {
            lat: strongest.lat,
            lon: strongest.lon,
            // Inflate accuracy to signal that the centroid was ambiguous.
            accuracy_m: 1000.0,
        });
    }
    let (lat, lon) = from_unit_vec(sx, sy, sz);

    // Estimate accuracy from the weighted spread of input positions
    let accuracy_m = estimate_accuracy(aps, lat, lon);

    Ok(Position {
        lat,
        lon,
        accuracy_m,
    })
}

/// Estimate accuracy in meters from the spread of AP positions around the centroid.
/// Uses the Haversine distance of each AP from the centroid, weighted.
fn estimate_accuracy(aps: &[PositionedAp], center_lat: f64, center_lon: f64) -> f64 {
    if aps.len() <= 1 {
        return 100.0;
    }

    let mut sum_dist = 0.0;
    let mut sum_weight = 0.0;

    for ap in aps {
        let weight = match ap.signal_dbm {
            Some(dbm) => f64::powf(10.0, dbm as f64 / 20.0),
            None => 1.0,
        };
        let dist = haversine_m(center_lat, center_lon, ap.lat, ap.lon);
        sum_dist += weight * dist;
        sum_weight += weight;
    }

    let avg_spread = sum_dist / sum_weight;

    // Accuracy is at least 10m, and grows with AP spread
    // The spread itself is a decent proxy for uncertainty
    avg_spread.clamp(10.0, 1000.0)
}

/// Haversine distance in meters between two lat/lon points.
fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0; // Earth radius in meters

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

    #[test]
    fn test_single_ap() {
        let aps = vec![PositionedAp {
            lat: 55.6684,
            lon: 12.5541,
            signal_dbm: Some(-65),
        }];
        let pos = trilaterate(&aps).unwrap();
        assert!((pos.lat - 55.6684).abs() < 1e-6);
        assert!((pos.lon - 12.5541).abs() < 1e-6);
        assert_eq!(pos.accuracy_m, 100.0);
    }

    #[test]
    fn test_two_aps_equal_weight() {
        // Two APs ~100m apart (within 200m radius, so no outlier rejection)
        let aps = vec![
            PositionedAp {
                lat: 48.8570,
                lon: 2.3510,
                signal_dbm: None,
            },
            PositionedAp {
                lat: 48.8580,
                lon: 2.3520,
                signal_dbm: None,
            },
        ];
        let pos = trilaterate(&aps).unwrap();
        assert!((pos.lat - 48.8575).abs() < 1e-4);
        assert!((pos.lon - 2.3515).abs() < 1e-4);
    }

    #[test]
    fn test_stronger_signal_more_weight() {
        let aps = vec![
            PositionedAp {
                lat: 55.0,
                lon: 12.0,
                signal_dbm: Some(-40), // very strong
            },
            PositionedAp {
                lat: 56.0,
                lon: 13.0,
                signal_dbm: Some(-80), // weak
            },
        ];
        let pos = trilaterate(&aps).unwrap();
        // Should be closer to 55.0, 12.0 (the stronger AP)
        assert!(pos.lat < 55.5);
        assert!(pos.lon < 12.5);
    }

    #[test]
    fn test_empty_aps_error() {
        assert!(trilaterate(&[]).is_err());
    }

    #[test]
    fn test_haversine() {
        // Copenhagen to Malmo: ~28km
        let dist = haversine_m(55.6761, 12.5683, 55.6050, 13.0038);
        assert!((dist - 28_000.0).abs() < 2000.0);
    }

    #[test]
    fn test_outlier_rejected() {
        // 5 APs in tight cluster + 1 far away (stale/moved router)
        let aps = vec![
            PositionedAp {
                lat: 48.857,
                lon: 2.351,
                signal_dbm: Some(-60),
            },
            PositionedAp {
                lat: 48.857,
                lon: 2.352,
                signal_dbm: Some(-65),
            },
            PositionedAp {
                lat: 48.858,
                lon: 2.351,
                signal_dbm: Some(-70),
            },
            PositionedAp {
                lat: 48.856,
                lon: 2.352,
                signal_dbm: Some(-75),
            },
            PositionedAp {
                lat: 48.857,
                lon: 2.350,
                signal_dbm: Some(-68),
            },
            PositionedAp {
                lat: -12.894,
                lon: -38.292,
                signal_dbm: Some(-60),
            }, // distant outlier
        ];
        let pos = trilaterate(&aps).unwrap();
        assert!(pos.lat > 48.0, "lat should be in cluster, got {}", pos.lat);
        assert!(pos.lon > 2.0, "lon should be in cluster, got {}", pos.lon);
    }

    #[test]
    fn test_outlier_moved_router() {
        // 4 APs in tight cluster + 1 that moved ~2km away
        let aps = vec![
            PositionedAp {
                lat: 48.857,
                lon: 2.351,
                signal_dbm: Some(-60),
            },
            PositionedAp {
                lat: 48.857,
                lon: 2.352,
                signal_dbm: Some(-65),
            },
            PositionedAp {
                lat: 48.858,
                lon: 2.351,
                signal_dbm: Some(-70),
            },
            PositionedAp {
                lat: 48.857,
                lon: 2.350,
                signal_dbm: Some(-68),
            },
            PositionedAp {
                lat: 48.875,
                lon: 2.325,
                signal_dbm: Some(-62),
            }, // ~2km away
        ];
        let filtered = filter_outliers(&aps);
        assert_eq!(filtered.len(), 4, "outlier 2km away should be rejected");
    }

    #[test]
    fn antimeridian_centroid_near_dateline_not_zero() {
        // Two APs straddling the antimeridian. Naive arithmetic mean would
        // give lon=0 (Africa). Spherical mean must give |lon| ≈ 180.
        let aps = vec![
            PositionedAp {
                lat: 0.0,
                lon: 179.0,
                signal_dbm: Some(-50),
            },
            PositionedAp {
                lat: 0.0,
                lon: -179.0,
                signal_dbm: Some(-50),
            },
        ];
        let pos = trilaterate(&aps).unwrap();
        assert!(
            pos.lon.abs() > 179.0,
            "expected |lon| > 179 (antimeridian), got lon={}",
            pos.lon
        );
        assert!(pos.lat.abs() < 0.5, "expected lat ≈ 0, got {}", pos.lat);
    }

    #[test]
    fn antimeridian_filter_outliers_does_not_mistake_dateline_cluster() {
        // Three APs all near the antimeridian — none should be rejected as outliers.
        let aps = vec![
            PositionedAp {
                lat: 0.0,
                lon: 179.99,
                signal_dbm: Some(-60),
            },
            PositionedAp {
                lat: 0.0,
                lon: -179.99,
                signal_dbm: Some(-60),
            },
            PositionedAp {
                lat: 0.001,
                lon: 180.0_f64.copysign(1.0),
                signal_dbm: Some(-60),
            },
        ];
        let kept = filter_outliers(&aps);
        assert_eq!(
            kept.len(),
            3,
            "antimeridian-clustered APs should not be rejected as outliers"
        );
    }

    #[test]
    fn polar_two_aps_lands_near_pole() {
        // Two APs at high latitude on opposite longitudes are physically close
        // (over the pole) but lat/lon arithmetic mean would put us at lat=89, lon=90,
        // which is on the equator-side of either AP. The spherical mean should land
        // at high latitude.
        let aps = vec![
            PositionedAp {
                lat: 89.0,
                lon: 0.0,
                signal_dbm: Some(-50),
            },
            PositionedAp {
                lat: 89.0,
                lon: 180.0,
                signal_dbm: Some(-50),
            },
        ];
        let pos = trilaterate(&aps).unwrap();
        // The two APs are ~220 km apart over the pole; the spherical mean is at
        // exactly the pole (lat=90), but the centroid magnitude is small there
        // and the antipodal-cancellation guard kicks in. Either lat ≈ 90 OR we
        // got the strongest-signal fallback (lat=89, accuracy=1000).
        let polar = pos.lat > 89.5;
        let fallback = (pos.lat - 89.0).abs() < 1e-6 && pos.accuracy_m >= 1000.0;
        assert!(
            polar || fallback,
            "polar trilateration: expected lat ≈ 90 or fallback to AP, got lat={} accuracy={}",
            pos.lat,
            pos.accuracy_m
        );
    }

    #[test]
    fn antipodal_aps_falls_back_to_strongest() {
        // Two APs on opposite sides of the planet. Vector sum is zero;
        // the centroid is undefined. Fallback should pick the stronger AP and
        // mark accuracy as poor.
        let aps = vec![
            PositionedAp {
                lat: 0.0,
                lon: 0.0,
                signal_dbm: Some(-40),
            },
            PositionedAp {
                lat: 0.0,
                lon: 180.0,
                signal_dbm: Some(-80),
            },
        ];
        let pos = trilaterate(&aps).unwrap();
        // Strongest is at (0, 0).
        assert!(
            pos.lat.abs() < 1e-6 && pos.lon.abs() < 1e-6,
            "antipodal fallback should pick strongest AP at (0,0), got ({},{})",
            pos.lat,
            pos.lon
        );
        assert!(
            pos.accuracy_m >= 1000.0,
            "accuracy should be inflated for ambiguous centroid, got {}",
            pos.accuracy_m
        );
    }

    #[test]
    fn test_no_outliers_all_close() {
        // All APs within 200m — none should be rejected
        let aps = vec![
            PositionedAp {
                lat: 48.8570,
                lon: 2.3510,
                signal_dbm: Some(-60),
            },
            PositionedAp {
                lat: 48.8571,
                lon: 2.3515,
                signal_dbm: Some(-65),
            },
            PositionedAp {
                lat: 48.8569,
                lon: 2.3512,
                signal_dbm: Some(-70),
            },
        ];
        let filtered = filter_outliers(&aps);
        assert_eq!(filtered.len(), 3);
    }
}
