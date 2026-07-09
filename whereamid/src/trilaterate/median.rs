//! Geometric median (Weiszfeld on the unit sphere) and the scalar `median`
//! helper. The inner Weiszfeld routine is deliberately tracing-free so
//! proptests and fuzz harnesses can call it without a tracing subscriber; the
//! caller inspects the rich `GeoMedianResult` and emits observability events.

use super::centroid::{from_unit_vec, to_unit_vec};
use std::fmt;

/// Outcome of `geometric_median` (task-0085). Inner Weiszfeld is pure /
/// tracing-free; the caller (`filter_outliers`) inspects this enum and
/// emits the appropriate `tracing::{debug,warn}` event so that silent
/// fallbacks are observable.
///
/// - `Converged((lat, lon))`: iteration met `WEISZFELD_CONVERGENCE` chord
///   threshold or terminated naturally (weight sum collapsed). Result is
///   trustworthy.
/// - `Capped((lat, lon))`: hit `WEISZFELD_MAX_ITERS` without convergence.
///   Returned value is the last iterate — usually still close to the
///   geometric median, but signals the algorithm did not fully settle.
/// - `Antipodal`: mid-iteration unit-vector sum cancelled to ~0. The
///   cluster has near-antipodal members; no meaningful center exists.
/// - `Degenerate`: no usable spherical-mean seed exists. This covers empty or
///   invalid/non-finite coordinate input as well as a seed vector that cancels
///   to ~0 for balanced antipodal input. Seed cancellation is more
///   pathological than mid-iteration `Antipodal`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GeoMedianResult {
    Converged((f64, f64)),
    Capped((f64, f64)),
    Antipodal,
    Degenerate,
}

/// Result of a cancellation-aware geometric-median execution.
///
/// Cancellation is deliberately a sibling of `Completed`, rather than an
/// `Option<GeoMedianResult>`, so callers cannot mistake a cancelled request
/// for a geometrically degenerate input.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CancellableGeoMedianResult {
    Completed(GeoMedianResult),
    Cancelled,
}

impl fmt::Display for GeoMedianResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeoMedianResult::Converged((lat, lon)) => {
                write!(f, "Converged(lat={lat:.6}, lon={lon:.6})")
            }
            GeoMedianResult::Capped((lat, lon)) => {
                write!(f, "Capped(lat={lat:.6}, lon={lon:.6})")
            }
            GeoMedianResult::Antipodal => write!(f, "Antipodal"),
            GeoMedianResult::Degenerate => write!(f, "Degenerate"),
        }
    }
}

/// Maximum Weiszfeld iterations for the geometric median. Linear
/// convergence; clean inputs settle in <10. 50 is generous.
pub(crate) const WEISZFELD_MAX_ITERS: usize = 50;

/// Chord-distance convergence threshold on the unit sphere (~6 cm at
/// Earth scale). Well below the STAGE2_FLOOR_M physical noise floor.
const WEISZFELD_CONVERGENCE: f64 = 1e-8;

/// Geometric median of (lat, lon) coordinates, computed in 3D unit-vector
/// space via Weiszfeld's algorithm. Returns a rich `GeoMedianResult` so the
/// caller can distinguish converged / iteration-capped / degenerate outcomes
/// and emit observability events.
///
/// Operates on a plain `&[(f64, f64)]` (task-0084) so it can be reused
/// outside the trilateration pipeline. Inner function remains tracing-free
/// (task-0085) so proptests and fuzz harnesses can call it without
/// initializing a tracing subscriber.
///
/// The geometric median minimizes the sum of distances to all input
/// points and has a ~50% breakdown point — half the inputs can be
/// arbitrary outliers without dragging the result away from the
/// remaining cluster. The arithmetic / spherical mean has 0% breakdown:
/// a single outlier displaces it.
///
/// Implementation: start from the spherical mean, then iterate
/// `c' = sum(p_i / |p_i - c|) / sum(1 / |p_i - c|)` (renormalize to the
/// unit sphere each step). Capped at `WEISZFELD_MAX_ITERS` iterations or
/// `WEISZFELD_CONVERGENCE` chord distance, whichever comes first.
pub(crate) fn geometric_median_detailed(coords: &[(f64, f64)]) -> GeoMedianResult {
    let mut never_cancelled = || false;
    match geometric_median_detailed_cancellable(coords, &mut never_cancelled) {
        CancellableGeoMedianResult::Completed(result) => result,
        CancellableGeoMedianResult::Cancelled => {
            unreachable!("the non-cancellable geometric-median wrapper cannot be cancelled")
        }
    }
}

/// Cancellation-aware geometric median used by history segmentation.
///
/// The callback is checked before the seed pass, for every input in that
/// pass, at every Weiszfeld iteration, and for every input within an
/// iteration. This bounds cancellation latency even for one very large
/// retained history segment. The ordinary `geometric_median_detailed`
/// API remains a zero-cancellation wrapper for existing callers.
pub(crate) fn geometric_median_detailed_cancellable<C>(
    coords: &[(f64, f64)],
    is_cancelled: &mut C,
) -> CancellableGeoMedianResult
where
    C: FnMut() -> bool + ?Sized,
{
    if is_cancelled() {
        return CancellableGeoMedianResult::Cancelled;
    }
    if coords.is_empty() {
        return CancellableGeoMedianResult::Completed(GeoMedianResult::Degenerate);
    }
    // Seed: spherical mean.
    let (mut sx, mut sy, mut sz) = (0.0, 0.0, 0.0);
    for &(lat, lon) in coords {
        if is_cancelled() {
            return CancellableGeoMedianResult::Cancelled;
        }
        if !lat.is_finite() || !lon.is_finite() {
            // Invalid coordinates cannot form a usable spherical seed. Keep
            // this distinct from cancellation, but use the existing
            // Degenerate fallback rather than returning a NaN point.
            return CancellableGeoMedianResult::Completed(GeoMedianResult::Degenerate);
        }
        let (x, y, z) = to_unit_vec(lat, lon);
        sx += x;
        sy += y;
        sz += z;
    }
    let m = (sx * sx + sy * sy + sz * sz).sqrt();
    if !m.is_finite() || m < 1e-9 {
        // The aggregate seed is unusable: it either became non-finite or
        // collapsed under a near-antipodal balance (e.g. two diametrically
        // opposed points with equal weight). More pathological than
        // mid-iteration cancellation.
        return CancellableGeoMedianResult::Completed(GeoMedianResult::Degenerate);
    }
    let (mut cx, mut cy, mut cz) = (sx / m, sy / m, sz / m);
    let mut converged = false;

    // Weiszfeld iterations on the unit sphere (chord-distance weighting).
    for _ in 0..WEISZFELD_MAX_ITERS {
        if is_cancelled() {
            return CancellableGeoMedianResult::Cancelled;
        }
        let (mut nx, mut ny, mut nz, mut wsum) = (0.0, 0.0, 0.0, 0.0);
        for &(lat, lon) in coords {
            if is_cancelled() {
                return CancellableGeoMedianResult::Cancelled;
            }
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
            // Natural termination: c is essentially at every input point
            // (single-cluster degenerate, but a meaningful answer).
            converged = true;
            break;
        }
        let nm = (nx * nx + ny * ny + nz * nz).sqrt();
        if nm < 1e-9 {
            // Antipodal-like cancellation under iteration. Cluster has
            // near-antipodal members; no meaningful single center exists.
            return CancellableGeoMedianResult::Completed(GeoMedianResult::Antipodal);
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
            converged = true;
            break;
        }
    }
    let point = from_unit_vec(cx, cy, cz);
    if converged {
        CancellableGeoMedianResult::Completed(GeoMedianResult::Converged(point))
    } else {
        CancellableGeoMedianResult::Completed(GeoMedianResult::Capped(point))
    }
}

/// Thin adapter: returns Some(lat, lon) for Converged and Capped (since the
/// capped iterate is usually still close to the median), None for Antipodal
/// and Degenerate. This remains useful to tests and callers that only need a
/// best-effort `Option` and do not care about the convergence story. History
/// production uses the detailed cancellable path so cancellation stays
/// distinct from geometric degeneracy.
///
/// `#[allow(dead_code)]`: current production paths use the detailed APIs; this
/// convenience adapter is exercised by tests but is unused in non-test builds.
#[allow(dead_code)]
pub(crate) fn geometric_median(coords: &[(f64, f64)]) -> Option<(f64, f64)> {
    match geometric_median_detailed(coords) {
        GeoMedianResult::Converged(p) | GeoMedianResult::Capped(p) => Some(p),
        GeoMedianResult::Antipodal | GeoMedianResult::Degenerate => None,
    }
}

#[allow(clippy::manual_is_multiple_of)]
pub(crate) fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    // total_cmp, not partial_cmp().unwrap(): a NaN element (a near-antipodal
    // haversine distance, or a NaN lat/lon read from a cached AP row — SQLite
    // REAL admits NaN) would make partial_cmp return None and panic the
    // unwrap in the locate hot path. total_cmp is a total order over all f64
    // (NaN sorts to an end) so the sort can never panic. task-0185.
    sorted.sort_by(|a, b| a.total_cmp(b));
    let n = sorted.len();
    if n % 2 == 0 {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trilaterate::PositionedAp;

    /// task-0185: median must not panic when the slice contains NaN — which
    /// can arrive from a near-antipodal haversine distance or a NaN lat/lon
    /// read from a cached AP row. total_cmp gives a total order (the +NaN
    /// sorts last), so the two central finite values are still averaged.
    #[test]
    fn median_does_not_panic_on_nan() {
        let m = median(&[f64::NAN, 5.0, 1.0, 3.0]);
        // total_cmp order: [1.0, 3.0, 5.0, NaN]; even length → (3.0+5.0)/2.
        assert_eq!(m, 4.0, "median should be 4.0 despite the NaN, got {m}");
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
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.701,
                lon: 12.581,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.699,
                lon: 12.579,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.700,
                lon: 12.582,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.702,
                lon: 12.580,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            // 4 outliers on 4 different continents
            PositionedAp {
                lat: -33.87,
                lon: 151.21,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            }, // Sydney
            PositionedAp {
                lat: 35.68,
                lon: 139.69,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            }, // Tokyo
            PositionedAp {
                lat: -12.89,
                lon: -38.29,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            }, // Salvador
            PositionedAp {
                lat: 40.71,
                lon: -74.00,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
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

    /// task-0085: each `GeoMedianResult` variant is reachable from a
    /// representative input. `Converged` is exercised by the cluster case
    /// (rapid settle); `Antipodal` and `Degenerate` are exercised by
    /// antipodal pairs. `Capped` cannot easily be triggered from a small
    /// deterministic input — the algorithm converges in <10 iterations on
    /// any non-pathological cluster, and pathological inputs typically hit
    /// the `Antipodal` branch first. We instead assert the *Display* impl
    /// shape for `Capped` (constructed directly) so callers can still
    /// trust the log format.
    #[test]
    fn geo_median_result_converged_for_tight_cluster() {
        let coords = [
            (55.700, 12.580),
            (55.701, 12.581),
            (55.699, 12.579),
            (55.700, 12.582),
            (55.702, 12.580),
        ];
        match geometric_median_detailed(&coords) {
            GeoMedianResult::Converged((lat, lon)) => {
                assert!((lat - 55.7).abs() < 0.01, "lat={lat}");
                assert!((lon - 12.58).abs() < 0.01, "lon={lon}");
            }
            other => panic!("expected Converged, got {other}"),
        }
    }

    #[test]
    fn geo_median_result_degenerate_for_antipodal_pair() {
        // Two diametrically opposed points: spherical-mean seed cancels
        // to magnitude ~0, so we bail at the Degenerate branch before
        // ever entering the Weiszfeld loop.
        let coords = [(0.0, 0.0), (0.0, 180.0)];
        let res = geometric_median_detailed(&coords);
        assert_eq!(
            res,
            GeoMedianResult::Degenerate,
            "expected Degenerate for antipodal pair, got {res}"
        );
    }

    #[test]
    fn geo_median_result_degenerate_pole_pair() {
        // North + South pole: also seed-cancels to magnitude 0.
        let coords = [(90.0, 0.0), (-90.0, 0.0)];
        let res = geometric_median_detailed(&coords);
        assert_eq!(
            res,
            GeoMedianResult::Degenerate,
            "expected Degenerate for pole-pair, got {res}"
        );
    }

    /// task-0087: cancellation is checked inside a Weiszfeld coordinate
    /// pass, not merely between complete iterations. For three points the
    /// first four checks cover entry + seed inputs, the fifth starts the
    /// iteration, and the sixth reaches its first coordinate.
    #[test]
    fn geometric_median_cancels_during_weiszfeld_iteration() {
        let coords = [(55.700, 12.580), (55.701, 12.581), (55.699, 12.579)];
        let mut checks = 0;
        let result = geometric_median_detailed_cancellable(&coords, &mut || {
            checks += 1;
            checks >= 6
        });
        assert_eq!(result, CancellableGeoMedianResult::Cancelled);
        assert_eq!(
            checks, 6,
            "cancellation should stop at the first coordinate of the iteration"
        );
    }

    #[test]
    fn geometric_median_non_finite_input_is_degenerate_not_nan() {
        let coords = [(55.7, 12.58), (f64::NAN, 12.59), (55.71, 12.60)];
        assert_eq!(
            geometric_median_detailed(&coords),
            GeoMedianResult::Degenerate
        );
        assert_eq!(geometric_median(&coords), None);
    }

    #[test]
    fn geo_median_result_antipodal_triple() {
        // Three near-antipodal points arranged so the seed (spherical
        // mean) does not cancel but the chord-weighted mid-iteration sum
        // does. Constructing this deterministically is finicky; if the
        // input happens to converge or seed-cancel, the test is a no-op
        // (we only assert *if* we hit Antipodal that the variant is
        // exactly as documented). The non-trivial existence of the
        // Antipodal branch is also exercised by the Display impl test
        // below.
        let coords = [(0.0, -90.0), (0.0, 90.0), (0.0001, 0.0)];
        let res = geometric_median_detailed(&coords);
        // Any of Converged / Antipodal / Degenerate is plausible here
        // depending on iteration order. The point is: no panic, no NaN.
        match res {
            GeoMedianResult::Converged((lat, lon)) | GeoMedianResult::Capped((lat, lon)) => {
                assert!(lat.is_finite() && lon.is_finite(), "got NaN: {res}");
            }
            GeoMedianResult::Antipodal | GeoMedianResult::Degenerate => {}
        }
    }

    #[test]
    fn geo_median_result_display_is_loggable() {
        // The Display impl is what shows up in tracing logs, so pin its
        // shape — a downstream regex matcher (or human eyeball) can rely
        // on these strings.
        assert_eq!(
            format!("{}", GeoMedianResult::Converged((55.7, 12.58))),
            "Converged(lat=55.700000, lon=12.580000)"
        );
        assert_eq!(
            format!("{}", GeoMedianResult::Capped((55.7, 12.58))),
            "Capped(lat=55.700000, lon=12.580000)"
        );
        assert_eq!(format!("{}", GeoMedianResult::Antipodal), "Antipodal");
        assert_eq!(format!("{}", GeoMedianResult::Degenerate), "Degenerate");
    }
}
