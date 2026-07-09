//! Two-stage robust outlier rejection: a centroid-independent pairwise-neighbor
//! pre-filter ("Brazil-catcher") followed by a geometric-median-centered
//! distance threshold. Both stages are documented in detail on
//! `filter_outliers`.

use super::median::{geometric_median_detailed, median, GeoMedianResult, WEISZFELD_MAX_ITERS};
use super::PositionedAp;
use crate::geo::haversine_m;

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
    // Mobile APs must be removed before even the small-input early return and,
    // critically, before drop_isolated: a cluster of train/bus BSSIDs must not
    // count as neighbors that save one another from stage 1.
    let fixed: Vec<PositionedAp> = aps.iter().copied().filter(|ap| !ap.is_mobile).collect();
    let aps = fixed.as_slice();
    if aps.len() <= 2 {
        return aps.to_vec();
    }

    // Marshal to coordinate slice once; stages 1 and 2 only need lat/lon.
    // The original PositionedAp slice is the source of truth for signal_dbm
    // — we thread indices through both stages so signal info survives.
    let coords: Vec<(f64, f64)> = aps.iter().map(|a| (a.lat, a.lon)).collect();
    let n_in = aps.len();

    // Stage 1: pairwise-neighbor pre-filter. Returns indices into `coords`
    // (and equivalently into `aps`). Empty result means every AP is
    // isolated — bypass stage 1 and feed all APs into stage 2.
    let mut survivor_idx = drop_isolated(&coords);
    if survivor_idx.is_empty() {
        tracing::debug!(
            target: "whereamid::trilaterate",
            n = n_in,
            "filter_outliers stage-1 bypass: every AP is isolated (> {NEIGHBOR_RADIUS_M:.0} m to nearest neighbor); using all inputs"
        );
        survivor_idx = (0..n_in).collect();
    }

    // After stage 1, may have ≤2 APs left — bypass stage 2 in that case.
    if survivor_idx.len() <= 2 {
        return survivor_idx.iter().map(|&i| aps[i]).collect();
    }

    let survivor_coords: Vec<(f64, f64)> = survivor_idx.iter().map(|&i| coords[i]).collect();

    // Stage 2: geometric-median centered threshold. Inspect the rich result
    // for observability (task-0085).
    let result = geometric_median_detailed(&survivor_coords);
    let (center_lat, center_lon) = match result {
        GeoMedianResult::Converged((lat, lon)) => (lat, lon),
        GeoMedianResult::Capped((lat, lon)) => {
            tracing::debug!(
                target: "whereamid::trilaterate",
                n = survivor_coords.len(),
                lat = lat,
                lon = lon,
                "filter_outliers: geometric_median hit iteration cap ({WEISZFELD_MAX_ITERS}) without convergence; using last iterate"
            );
            (lat, lon)
        }
        GeoMedianResult::Antipodal => {
            tracing::warn!(
                target: "whereamid::trilaterate",
                n = survivor_coords.len(),
                "filter_outliers: geometric_median produced {result} (mid-iteration cancellation); keeping stage-1 survivors as-is"
            );
            return survivor_idx.iter().map(|&i| aps[i]).collect();
        }
        GeoMedianResult::Degenerate => {
            tracing::warn!(
                target: "whereamid::trilaterate",
                n = survivor_coords.len(),
                "filter_outliers: geometric_median produced {result} (unusable seed: invalid/non-finite input or antipodal cancellation); keeping stage-1 survivors as-is"
            );
            return survivor_idx.iter().map(|&i| aps[i]).collect();
        }
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
        .map(|(&i, _)| aps[i])
        .collect();

    if kept.is_empty() {
        // Should be very rare: every survivor was outside the threshold
        // band around its own geometric median. Possible if the threshold
        // formula under-bounds an extreme cluster spread. Fall back to the
        // stage-1 survivors so trilaterate has something to chew on.
        tracing::debug!(
            target: "whereamid::trilaterate",
            n = survivor_coords.len(),
            threshold_m = threshold,
            median_dist_m = median_dist,
            "filter_outliers stage-2 kept nothing; falling back to stage-1 survivors"
        );
        survivor_idx.iter().map(|&i| aps[i]).collect()
    } else {
        kept
    }
}

/// Stage 1 of outlier filtering: return indices of points that have at least
/// one neighbor within `NEIGHBOR_RADIUS_M`. Independent of any centroid, so
/// robust to even a majority of catastrophic outliers as long as a real
/// cluster exists.
///
/// Returns an empty `Vec` if every input point is isolated — the caller
/// must decide how to handle that bypass (typically: treat all inputs as
/// survivors so stage 2 still has data). The previous version hid this
/// fallback inside the function; exposing it lets `filter_outliers` log
/// the bypass for observability (task-0085).
///
/// Returning indices (rather than coordinates) lets callers preserve any
/// per-point metadata attached to the original slice — e.g. signal_dbm in
/// `filter_outliers` — without a separate join step.
fn drop_isolated(coords: &[(f64, f64)]) -> Vec<usize> {
    if coords.len() <= 1 {
        return (0..coords.len()).collect();
    }
    coords
        .iter()
        .enumerate()
        .filter(|(i, &(lat, lon))| {
            coords.iter().enumerate().any(|(j, &(olat, olon))| {
                i != &j && haversine_m(lat, lon, olat, olon) <= NEIGHBOR_RADIUS_M
            })
        })
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outlier_moved_router() {
        // 4 APs in tight cluster + 1 that moved ~2km away
        let aps = vec![
            PositionedAp {
                lat: 48.857,
                lon: 2.351,
                signal_dbm: Some(-60),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.857,
                lon: 2.352,
                signal_dbm: Some(-65),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.858,
                lon: 2.351,
                signal_dbm: Some(-70),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.857,
                lon: 2.350,
                signal_dbm: Some(-68),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.875,
                lon: 2.325,
                signal_dbm: Some(-62),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            }, // ~2km away
        ];
        let filtered = filter_outliers(&aps);
        assert_eq!(filtered.len(), 4, "outlier 2km away should be rejected");
    }

    #[test]
    fn antimeridian_filter_outliers_does_not_mistake_dateline_cluster() {
        // Three APs all near the antimeridian — none should be rejected as outliers.
        let aps = vec![
            PositionedAp {
                lat: 0.0,
                lon: 179.99,
                signal_dbm: Some(-60),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.0,
                lon: -179.99,
                signal_dbm: Some(-60),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.001,
                lon: 180.0_f64.copysign(1.0),
                signal_dbm: Some(-60),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
        ];
        let kept = filter_outliers(&aps);
        assert_eq!(
            kept.len(),
            3,
            "antimeridian-clustered APs should not be rejected as outliers"
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
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.708,
                lon: 12.585,
                signal_dbm: Some(-72),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.706,
                lon: 12.586,
                signal_dbm: Some(-74),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            // Brazil — no neighbor within NEIGHBOR_RADIUS_M.
            PositionedAp {
                lat: -12.894,
                lon: -38.292,
                signal_dbm: Some(-49),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
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

    /// task-0083: pin stage-1's "everyone is isolated" detection.
    /// task-0085: drop_isolated now returns empty when every AP is
    /// isolated (so filter_outliers can log the bypass); the bypass
    /// itself is handled at the call site and verified via
    /// `filter_outliers_all_isolated_bypasses_stage_1` below.
    #[test]
    fn drop_isolated_returns_empty_when_all_isolated() {
        let coords = [(55.0, 12.0), (55.5, 12.5), (56.0, 13.0)];
        let kept = drop_isolated(&coords);
        assert!(
            kept.is_empty(),
            "drop_isolated must return empty when every point is isolated; got {kept:?}"
        );
    }

    /// task-0085: the bypass semantics — `filter_outliers` must keep all
    /// inputs when stage 1 returns empty, so stage 2 has data to operate
    /// on. This is the externally-observable property the old
    /// `drop_isolated_falls_back_when_all_isolated` test asserted.
    #[test]
    fn filter_outliers_all_isolated_bypasses_stage_1() {
        let aps = [
            PositionedAp {
                lat: 55.0,
                lon: 12.0,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.5,
                lon: 12.5,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 56.0,
                lon: 13.0,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
        ];
        let kept = filter_outliers(&aps);
        assert_eq!(
            kept.len(),
            3,
            "filter_outliers must bypass stage 1 (not erase) when every AP is isolated"
        );
    }

    /// task-0085 AC #6: filter_outliers emits a tracing event when it
    /// traverses a silent-fallback path. The Brazil-in-Copenhagen scenario
    /// is the *success* path (no fallback), so we instead verify on the
    /// Degenerate-input case, which is one of the five fallback points.
    /// The original AC #6 wording assumed Brazil hit a fallback; it does
    /// not. Verified manually that the Brazil test under
    /// `RUST_LOG=whereamid::trilaterate=debug cargo test brazil` emits
    /// nothing (success path), and that this test captures the warn for
    /// the antipodal/degenerate path — which is the substantive intent
    /// (silent-fallback paths self-report).
    #[test]
    fn filter_outliers_emits_tracing_on_degenerate_fallback() {
        // task-0255: shared `test_support::capture` sink records (level,
        // target, message) for every event and installs the permissive global
        // (task-0251) so the scoped capture reliably sees its events. This site
        // filters on `target` — the assertion below is unchanged.
        let (_, events) = crate::test_support::capture(|| {
            // Antipodal pair plus an offset point: 3 APs so filter_outliers
            // does not short-circuit at the n<=2 fast path. The first two
            // are antipodal, the third is on the equator. Stage 1 likely
            // drops the antipodal pair (no neighbor within 2 km), leaving
            // one survivor → returns early without invoking
            // geometric_median. We instead hand-craft an input where stage
            // 1 keeps all three and stage 2 hits Degenerate.
            let aps = [
                PositionedAp {
                    lat: 0.0,
                    lon: 0.0,
                    signal_dbm: Some(-60),
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 0.0,
                    lon: 0.0001,
                    signal_dbm: Some(-60),
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 0.0,
                    lon: 180.0,
                    signal_dbm: Some(-60),
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    is_mobile: false,
                },
            ];
            // Stage 1: the antipodal point at (0, 180) is >2 km from both
            // (0,0) and (0, 0.0001), so it's dropped. Survivors = 2 →
            // stage-2 bypass. So this hits no fallback log. We need an
            // input where stage 1 keeps every AP AND stage 2 is degenerate.
            //
            // Easier: invoke filter_outliers on inputs that go through
            // stage 2 and Degenerate. Add a neighbor for the antipodal
            // point so stage 1 keeps it.
            let _ = filter_outliers(&aps);

            // The targeted scenario: two exactly-antipodal pairs. Each
            // pair's intra-pair distance is 0 (so stage 1 keeps every AP),
            // but the spherical-mean seed cancels to magnitude 0 →
            // stage 2 returns Degenerate, which is the warn fallback.
            let aps_deg = [
                PositionedAp {
                    lat: 0.0,
                    lon: 0.0,
                    signal_dbm: Some(-60),
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 0.0,
                    lon: 0.0,
                    signal_dbm: Some(-60),
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 0.0,
                    lon: 180.0,
                    signal_dbm: Some(-60),
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 0.0,
                    lon: 180.0,
                    signal_dbm: Some(-60),
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    is_mobile: false,
                },
            ];
            let _ = filter_outliers(&aps_deg);
        });

        // Assert: at least one event was emitted with the expected
        // target. We don't pin the exact message text — that's free to
        // evolve — but we do require it came from
        // whereamid::trilaterate.
        let from_trilaterate: Vec<_> = events
            .iter()
            .filter(|(_, target, _)| target == "whereamid::trilaterate")
            .collect();
        assert!(
            !from_trilaterate.is_empty(),
            "expected at least one tracing event from whereamid::trilaterate; got all events: {events:?}"
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
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.8571,
                lon: 2.3515,
                signal_dbm: Some(-65),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.8569,
                lon: 2.3512,
                signal_dbm: Some(-70),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
        ];
        let filtered = filter_outliers(&aps);
        assert_eq!(filtered.len(), 3);
    }
}
