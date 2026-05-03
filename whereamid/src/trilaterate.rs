//! Weighted centroid trilateration from AP positions and signal strengths.

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

/// Filter out outlier APs that are implausibly far from the cluster.
///
/// Algorithm:
/// 1. Compute median lat/lon (robust center estimate)
/// 2. Compute each AP's distance from that median
/// 3. Compute the median of those distances (MAD-like measure of cluster spread)
/// 4. Reject APs whose distance from median exceeds max(200m, 3 * median_distance)
///
/// The 200m floor reflects the physical assumption that APs in a neighborhood
/// are within ~200m. The 3x median_distance handles cases where all APs are
/// further apart (e.g. rural, or all have somewhat stale positions) — it adapts
/// to the actual data spread rather than rejecting everything.
pub fn filter_outliers(aps: &[PositionedAp]) -> Vec<PositionedAp> {
    if aps.len() <= 2 {
        return aps.to_vec();
    }

    // Compute median lat/lon
    let median_lat = median(&aps.iter().map(|a| a.lat).collect::<Vec<_>>());
    let median_lon = median(&aps.iter().map(|a| a.lon).collect::<Vec<_>>());

    // Compute each AP's distance from the median
    let distances: Vec<f64> = aps
        .iter()
        .map(|ap| haversine_m(median_lat, median_lon, ap.lat, ap.lon))
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

    let mut sum_lat = 0.0;
    let mut sum_lon = 0.0;
    let mut sum_weight = 0.0;

    for ap in aps {
        let weight = match ap.signal_dbm {
            Some(dbm) => f64::powf(10.0, dbm as f64 / 20.0),
            None => 1.0,
        };
        sum_lat += weight * ap.lat;
        sum_lon += weight * ap.lon;
        sum_weight += weight;
    }

    let lat = sum_lat / sum_weight;
    let lon = sum_lon / sum_weight;

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
                lat: 55.7070,
                lon: 12.5850,
                signal_dbm: None,
            },
            PositionedAp {
                lat: 55.7080,
                lon: 12.5860,
                signal_dbm: None,
            },
        ];
        let pos = trilaterate(&aps).unwrap();
        assert!((pos.lat - 55.7075).abs() < 1e-4);
        assert!((pos.lon - 12.5855).abs() < 1e-4);
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
        // 5 APs in Copenhagen cluster + 1 in Brazil
        let aps = vec![
            PositionedAp {
                lat: 55.707,
                lon: 12.585,
                signal_dbm: Some(-60),
            },
            PositionedAp {
                lat: 55.707,
                lon: 12.586,
                signal_dbm: Some(-65),
            },
            PositionedAp {
                lat: 55.708,
                lon: 12.585,
                signal_dbm: Some(-70),
            },
            PositionedAp {
                lat: 55.706,
                lon: 12.586,
                signal_dbm: Some(-75),
            },
            PositionedAp {
                lat: 55.707,
                lon: 12.584,
                signal_dbm: Some(-68),
            },
            PositionedAp {
                lat: -12.894,
                lon: -38.292,
                signal_dbm: Some(-60),
            }, // Brazil outlier
        ];
        let pos = trilaterate(&aps).unwrap();
        // Result should be in Copenhagen, not pulled toward Brazil
        assert!(
            pos.lat > 55.0,
            "lat should be in Copenhagen, got {}",
            pos.lat
        );
        assert!(
            pos.lon > 12.0,
            "lon should be in Copenhagen, got {}",
            pos.lon
        );
    }

    #[test]
    fn test_outlier_moved_router() {
        // 4 APs in tight cluster + 1 that moved 2.5km away (stale WiGLE data)
        let aps = vec![
            PositionedAp {
                lat: 55.707,
                lon: 12.585,
                signal_dbm: Some(-60),
            },
            PositionedAp {
                lat: 55.707,
                lon: 12.586,
                signal_dbm: Some(-65),
            },
            PositionedAp {
                lat: 55.708,
                lon: 12.585,
                signal_dbm: Some(-70),
            },
            PositionedAp {
                lat: 55.707,
                lon: 12.584,
                signal_dbm: Some(-68),
            },
            PositionedAp {
                lat: 55.715,
                lon: 12.559,
                signal_dbm: Some(-62),
            }, // ~2km away (RouterX)
        ];
        let filtered = filter_outliers(&aps);
        assert_eq!(filtered.len(), 4, "outlier 2km away should be rejected");
    }

    #[test]
    fn test_no_outliers_all_close() {
        // All APs within 200m — none should be rejected
        let aps = vec![
            PositionedAp {
                lat: 55.7070,
                lon: 12.5850,
                signal_dbm: Some(-60),
            },
            PositionedAp {
                lat: 55.7071,
                lon: 12.5855,
                signal_dbm: Some(-65),
            },
            PositionedAp {
                lat: 55.7069,
                lon: 12.5852,
                signal_dbm: Some(-70),
            },
        ];
        let filtered = filter_outliers(&aps);
        assert_eq!(filtered.len(), 3);
    }
}
