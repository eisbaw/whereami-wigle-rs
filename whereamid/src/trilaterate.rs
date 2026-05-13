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

/// Pairwise-neighbor radius for stage 1 of outlier filtering (task-0083).
/// An AP must have at least one other AP within this distance to survive;
/// catches catastrophic outliers (wrong continent, stale cache from a
/// different city) regardless of how poisoned the centroid would be.
const NEIGHBOR_RADIUS_M: f64 = 2_000.0;

/// Stage-2 absolute floor on the distance threshold (meters). Even when
/// the cluster is unrealistically tight, no AP within this radius of the
/// geometric median is treated as an outlier. Reflects typical AP radio
/// reach (~50–200m).
const STAGE2_FLOOR_M: f64 = 200.0;

/// Stage-2 multiplier on the median distance from the geometric median.
/// 3× adapts the threshold to the actual cluster spread without being so
/// permissive that single outliers stretch it past their own distance.
const STAGE2_MEDIAN_MULTIPLIER: f64 = 3.0;

/// Maximum Weiszfeld iterations for the geometric median. Linear
/// convergence; clean inputs settle in <10. 50 is generous.
const WEISZFELD_MAX_ITERS: usize = 50;

/// Chord-distance convergence threshold on the unit sphere (~6 cm at
/// Earth scale). Well below the STAGE2_FLOOR_M physical noise floor.
const WEISZFELD_CONVERGENCE: f64 = 1e-8;

/// Filter out outlier APs that are implausibly far from the cluster.
///
/// Two-stage robust filter (task-0083). The prior single-stage filter used
/// an *unweighted spherical mean* as the cluster center, which has a 0%
/// breakdown point: a single catastrophic outlier (e.g. WiGLE returning a
/// Brazilian position for a Copenhagen-visible BSSID) pulls the center
/// thousands of km away, inflating the median and the threshold so the
/// poisoning outlier can never be rejected. Replaced with:
///
/// 1. **Pairwise neighbor sanity** ("Brazil-catcher"): each AP must have
///    at least one other AP within `NEIGHBOR_RADIUS_M`. This decision is
///    independent of any centroid, so a single far-away outlier cannot
///    defeat it. If applying this would drop *every* AP (rural cluster
///    where all APs are >2 km from each other), the stage is bypassed.
/// 2. **Geometric-median based threshold**: compute the geometric median
///    of the survivors (Weiszfeld iterations on unit-vector representation;
///    ~50% breakdown vs the spherical mean's 0%). Then reject survivors
///    whose distance from the geometric median exceeds
///    `max(200m, 3 * median_dist)`. The 200m floor reflects the physical
///    assumption that APs in a neighborhood are within ~200m. The 3×
///    multiplier adapts to actual cluster spread.
pub fn filter_outliers(aps: &[PositionedAp]) -> Vec<PositionedAp> {
    if aps.len() <= 2 {
        return aps.to_vec();
    }

    // Marshal to coordinate slice once; stages 1 and 2 only need lat/lon.
    // The original PositionedAp slice is the source of truth for signal_dbm
    // — we thread indices through both stages so signal info survives.
    let coords: Vec<(f64, f64)> = aps.iter().map(|a| (a.lat, a.lon)).collect();

    // Stage 1: pairwise-neighbor pre-filter. Returns indices into `coords`
    // (and equivalently into `aps`).
    let survivor_idx = drop_isolated(&coords);

    // After stage 1, may have ≤2 APs left — bypass stage 2 in that case.
    if survivor_idx.len() <= 2 {
        return survivor_idx.iter().map(|&i| aps[i].clone()).collect();
    }

    let survivor_coords: Vec<(f64, f64)> = survivor_idx.iter().map(|&i| coords[i]).collect();

    // Stage 2: geometric-median centered threshold.
    let (center_lat, center_lon) = match geometric_median(&survivor_coords) {
        Some(c) => c,
        // Degenerate (e.g. antipodal survivors); keep what stage 1 left.
        None => return survivor_idx.iter().map(|&i| aps[i].clone()).collect(),
    };

    let distances: Vec<f64> = survivor_coords
        .iter()
        .map(|&(lat, lon)| haversine_m(center_lat, center_lon, lat, lon))
        .collect();

    let median_dist = median(&distances);
    let threshold = f64::max(STAGE2_FLOOR_M, STAGE2_MEDIAN_MULTIPLIER * median_dist);

    let kept: Vec<PositionedAp> = survivor_idx
        .iter()
        .zip(distances.iter())
        .filter(|(_, d)| **d <= threshold)
        .map(|(&i, _)| aps[i].clone())
        .collect();

    if kept.is_empty() {
        survivor_idx.iter().map(|&i| aps[i].clone()).collect()
    } else {
        kept
    }
}

/// Stage 1 of outlier filtering: return indices of points that have at least
/// one neighbor within `NEIGHBOR_RADIUS_M`. Independent of any centroid, so
/// robust to even a majority of catastrophic outliers as long as a real
/// cluster exists.
///
/// Fallback: if applying the filter would drop every point (truly sparse
/// rural cluster where each point is >2 km from any other), all indices are
/// returned so the caller (stage 2) still has data.
///
/// Returning indices (rather than coordinates) lets callers preserve any
/// per-point metadata attached to the original slice — e.g. signal_dbm in
/// `filter_outliers` — without a separate join step.
fn drop_isolated(coords: &[(f64, f64)]) -> Vec<usize> {
    if coords.len() <= 1 {
        return (0..coords.len()).collect();
    }
    let kept: Vec<usize> = coords
        .iter()
        .enumerate()
        .filter(|(i, &(lat, lon))| {
            coords.iter().enumerate().any(|(j, &(olat, olon))| {
                i != &j && haversine_m(lat, lon, olat, olon) <= NEIGHBOR_RADIUS_M
            })
        })
        .map(|(i, _)| i)
        .collect();
    if kept.is_empty() {
        (0..coords.len()).collect()
    } else {
        kept
    }
}

/// Geometric median of (lat, lon) coordinates, computed in 3D unit-vector
/// space via Weiszfeld's algorithm. Returns `(lat, lon)` in degrees, or
/// `None` if the cluster is too degenerate to produce a meaningful center
/// (antipodal cancellation).
///
/// Operates on a plain `&[(f64, f64)]` (task-0084) so it can be reused
/// outside the trilateration pipeline — e.g. history.rs stay-point
/// centroids — without coupling to `PositionedAp`.
///
/// The geometric median minimizes the sum of distances to all input
/// points and has a ~50% breakdown point — half the inputs can be
/// arbitrary outliers without dragging the result away from the
/// remaining cluster. The arithmetic / spherical mean has 0% breakdown:
/// a single outlier displaces it.
///
/// Implementation: start from the spherical mean, then iterate
/// `c' = sum(p_i / |p_i - c|) / sum(1 / |p_i - c|)` (renormalize to the
/// unit sphere each step). Capped at 50 iterations or 1e-8 chord
/// convergence, whichever comes first.
pub(crate) fn geometric_median(coords: &[(f64, f64)]) -> Option<(f64, f64)> {
    if coords.is_empty() {
        return None;
    }
    // Seed: spherical mean.
    let (mut sx, mut sy, mut sz) = (0.0, 0.0, 0.0);
    for &(lat, lon) in coords {
        let (x, y, z) = to_unit_vec(lat, lon);
        sx += x;
        sy += y;
        sz += z;
    }
    let m = (sx * sx + sy * sy + sz * sz).sqrt();
    if m < 1e-9 {
        return None;
    }
    let (mut cx, mut cy, mut cz) = (sx / m, sy / m, sz / m);

    // Weiszfeld iterations on the unit sphere (chord-distance weighting).
    for _ in 0..WEISZFELD_MAX_ITERS {
        let (mut nx, mut ny, mut nz, mut wsum) = (0.0, 0.0, 0.0, 0.0);
        for &(lat, lon) in coords {
            let (x, y, z) = to_unit_vec(lat, lon);
            let dx = x - cx;
            let dy = y - cy;
            let dz = z - cz;
            let d = (dx * dx + dy * dy + dz * dz).sqrt();
            // Cap weight to avoid singular spike when c coincides with a point.
            let w = if d < 1e-9 { 1e9 } else { 1.0 / d };
            nx += w * x;
            ny += w * y;
            nz += w * z;
            wsum += w;
        }
        if wsum < 1e-15 {
            break;
        }
        let nm = (nx * nx + ny * ny + nz * nz).sqrt();
        if nm < 1e-9 {
            // Antipodal-like cancellation under iteration; bail with prior c.
            break;
        }
        let (ncx, ncy, ncz) = (nx / nm, ny / nm, nz / nm);
        let dx = ncx - cx;
        let dy = ncy - cy;
        let dz = ncz - cz;
        let delta = (dx * dx + dy * dy + dz * dz).sqrt();
        cx = ncx;
        cy = ncy;
        cz = ncz;
        if delta < WEISZFELD_CONVERGENCE {
            break;
        }
    }
    Some(from_unit_vec(cx, cy, cz))
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

// task-0081: haversine_m moved to crate::geo. Use it via the path below.
use crate::geo::haversine_m;

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

    /// task-0083: pin stage-1 behavior directly so a refactor of
    /// `filter_outliers` can't silently lose the Brazil-catcher property.
    #[test]
    fn drop_isolated_drops_lone_outlier() {
        let aps = [
            PositionedAp {
                lat: 55.707,
                lon: 12.585,
                signal_dbm: Some(-70),
            },
            PositionedAp {
                lat: 55.708,
                lon: 12.585,
                signal_dbm: Some(-72),
            },
            PositionedAp {
                lat: 55.706,
                lon: 12.586,
                signal_dbm: Some(-74),
            },
            // Brazil — no neighbor within NEIGHBOR_RADIUS_M.
            PositionedAp {
                lat: -12.894,
                lon: -38.292,
                signal_dbm: Some(-49),
            },
        ];
        let coords: Vec<(f64, f64)> = aps.iter().map(|a| (a.lat, a.lon)).collect();
        let kept = drop_isolated(&coords);
        assert_eq!(kept.len(), 3, "the isolated AP must be dropped");
        for &i in &kept {
            assert!(
                aps[i].lat > 0.0,
                "no Southern-hemisphere survivors expected"
            );
        }
    }

    /// task-0083: pin stage-1's "everyone is isolated -> bypass" fallback.
    /// A 3-AP cluster with all pairwise distances > NEIGHBOR_RADIUS_M
    /// would otherwise be wiped, leaving stage 2 with nothing to chew on.
    #[test]
    fn drop_isolated_falls_back_when_all_isolated() {
        let aps = [
            PositionedAp {
                lat: 55.0,
                lon: 12.0,
                signal_dbm: None,
            },
            PositionedAp {
                lat: 55.5,
                lon: 12.5,
                signal_dbm: None,
            },
            PositionedAp {
                lat: 56.0,
                lon: 13.0,
                signal_dbm: None,
            },
        ];
        let coords: Vec<(f64, f64)> = aps.iter().map(|a| (a.lat, a.lon)).collect();
        let kept = drop_isolated(&coords);
        assert_eq!(
            kept.len(),
            3,
            "stage-1 must bypass (not erase) when every AP is isolated"
        );
    }

    /// task-0083: direct test of the geometric median. With 5 clustered
    /// points and 4 scattered outliers spread across the planet, the
    /// geometric median's ~50% breakdown should still pin the result to
    /// the cluster.
    #[test]
    fn geometric_median_resists_minority_outliers() {
        let aps = vec![
            // Cluster of 5 near (55.7, 12.58)
            PositionedAp {
                lat: 55.700,
                lon: 12.580,
                signal_dbm: None,
            },
            PositionedAp {
                lat: 55.701,
                lon: 12.581,
                signal_dbm: None,
            },
            PositionedAp {
                lat: 55.699,
                lon: 12.579,
                signal_dbm: None,
            },
            PositionedAp {
                lat: 55.700,
                lon: 12.582,
                signal_dbm: None,
            },
            PositionedAp {
                lat: 55.702,
                lon: 12.580,
                signal_dbm: None,
            },
            // 4 outliers on 4 different continents
            PositionedAp {
                lat: -33.87,
                lon: 151.21,
                signal_dbm: None,
            }, // Sydney
            PositionedAp {
                lat: 35.68,
                lon: 139.69,
                signal_dbm: None,
            }, // Tokyo
            PositionedAp {
                lat: -12.89,
                lon: -38.29,
                signal_dbm: None,
            }, // Salvador
            PositionedAp {
                lat: 40.71,
                lon: -74.00,
                signal_dbm: None,
            }, // NYC
        ];
        let coords: Vec<(f64, f64)> = aps.iter().map(|a| (a.lat, a.lon)).collect();
        let (lat, lon) = geometric_median(&coords).expect("non-degenerate");
        // Cluster is at 55.7N, 12.58E. Geometric median should be within
        // ~1° (~110 km) — the spherical mean would land in the Atlantic.
        assert!(
            (lat - 55.7).abs() < 1.0,
            "geometric median lat should be near cluster (55.7), got {lat}"
        );
        assert!(
            (lon - 12.58).abs() < 1.0,
            "geometric median lon should be near cluster (12.58), got {lon}"
        );
    }

    /// task-0083: real-world incident. The user was at Strandboulevarden 95
    /// in Copenhagen; `whereami locate` returned (55.71, 12.57) ±916m, ~900m
    /// off, at Drejøgade. Cause: WiGLE had cached BSSID F6:B1:9C:0A:3A:60
    /// (a randomized client MAC) at (-12.89, -38.29) — Salvador, Brazil.
    /// The old single-stage filter (spherical-mean centroid + median
    /// distance threshold) failed because the Brazilian outlier pulled
    /// the centroid into northern France, inflating the median past the
    /// threshold needed to reject the outlier itself. The new two-stage
    /// filter must drop the Brazilian AP and land in the Copenhagen
    /// cluster regardless.
    #[test]
    fn brazil_in_copenhagen_incident() {
        let aps = vec![
            // 6 Copenhagen APs around Strandboulevarden 95
            PositionedAp {
                lat: 55.70696,
                lon: 12.58566,
                signal_dbm: Some(-76),
            },
            PositionedAp {
                lat: 55.70709,
                lon: 12.58565,
                signal_dbm: Some(-72),
            },
            PositionedAp {
                lat: 55.70735,
                lon: 12.58569,
                signal_dbm: Some(-73),
            },
            PositionedAp {
                lat: 55.70714,
                lon: 12.58544,
                signal_dbm: Some(-73),
            },
            PositionedAp {
                lat: 55.70713,
                lon: 12.58544,
                signal_dbm: Some(-73),
            },
            PositionedAp {
                lat: 55.70662,
                lon: 12.58570,
                signal_dbm: Some(-77),
            },
            // 1 Brazilian outlier — STRONGEST signal, so without filtering
            // it would dominate the weighted centroid.
            PositionedAp {
                lat: -12.89422,
                lon: -38.29226,
                signal_dbm: Some(-49),
            },
        ];
        let kept = filter_outliers(&aps);
        assert_eq!(
            kept.len(),
            6,
            "Brazilian outlier must be dropped; got {} survivors",
            kept.len()
        );
        for ap in &kept {
            assert!(
                ap.lat > 55.0 && ap.lon > 12.0,
                "all survivors must be in Copenhagen cluster, got ({}, {})",
                ap.lat,
                ap.lon
            );
        }
        let pos = trilaterate(&aps).unwrap();
        assert!(
            (pos.lat - 55.707).abs() < 0.01,
            "centroid lat should be near 55.707, got {}",
            pos.lat
        );
        assert!(
            (pos.lon - 12.585).abs() < 0.01,
            "centroid lon should be near 12.585, got {}",
            pos.lon
        );
        assert!(
            pos.accuracy_m < 100.0,
            "accuracy should be ~10s of meters, got {}",
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
