//! Weighted centroid trilateration from AP positions and signal strengths.
//!
//! Centroid math operates on the **unit sphere**, not on raw lat/lon. This
//! avoids the antimeridian discontinuity (an arithmetic mean of lon=+179 and
//! lon=-179 yields lon=0, which is on the wrong side of the planet) and the
//! polar singularity (longitude is degenerate near the poles). Each (lat, lon)
//! is mapped to a unit 3-vector, vectors are weighted-summed and normalized,
//! then mapped back to (lat, lon) via atan2/asin.
//!
//! The module is split into three concern-focused submodules; this file owns
//! the public data types (`PositionedAp`, `Position`), the vote-weight math,
//! the `trilaterate` orchestrator, and the `estimate_accuracy` helper, and it
//! re-exports the pieces callers reach for so the external API path
//! (`crate::trilaterate::X`) is unchanged.

mod centroid;
mod median;
mod outliers;

pub use centroid::ap_consensus_score;
pub(crate) use centroid::{from_unit_vec, to_unit_vec};
use median::median;
pub use outliers::filter_outliers;

// These re-exports preserve the pre-split `crate::trilaterate::X` API path for
// the geometric-median API. Their only consumers are the `whereamid` binary
// (`history.rs` segmentation) and tests, not the library itself, so the lib-only
// compilation sees them as unused — the same reason `geometric_median` carries
// `#[allow(dead_code)]`. Keeping the re-export is required so nothing outside
// this module breaks (task-0146 AC#4).
#[allow(unused_imports)]
pub(crate) use median::{
    geometric_median, geometric_median_detailed_cancellable, CancellableGeoMedianResult,
    GeoMedianResult,
};

use crate::geo::haversine_m;
use crate::rssi::{Rssi, RssiSource};
use anyhow::{bail, Result};

/// Input for trilateration: a positioned AP with optional signal.
///
/// `Copy` (task-0086): every field is itself `Copy` (f64, f64, Option<i32>,
/// bool, and the `Copy` `RssiSource` enum), so cloning is already a bitwise
/// memcpy. Deriving `Copy` removes the ergonomic noise of explicit `.clone()`
/// calls. Caveat: this is a soft promise to consumers — if a future field is
/// non-Copy (e.g. an owning SSID `String`), reverting forces every by-value
/// call site to clone or take a reference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedAp {
    pub lat: f64,
    pub lon: f64,
    /// Raw signal strength in dBm (still an `i32` because the cache/debounce
    /// path it arrives through is `i32`). Pair with `rssi_source` to know how
    /// this value was obtained.
    pub signal_dbm: Option<i32>,
    /// Provenance of `signal_dbm`: iw authoritative raw dBm vs nmcli's biased
    /// percentage-linearization (see [`crate::rssi`]). Threaded from the
    /// scanner so trilateration knows the measurement source. The current
    /// vote-weight math ignores it (weights are source-agnostic); it exists so
    /// a future task MAY down-weight or refuse to mix sources — task-0126 does
    /// NOT change any weighting behavior.
    pub rssi_source: crate::rssi::RssiSource,
    /// Persisted scanner-SSID mobility classification. Mobile APs are never
    /// allowed to participate in outlier clustering or centroid math.
    pub is_mobile: bool,
}

/// Trilateration result.
#[derive(Debug, Clone)]
pub struct Position {
    pub lat: f64,
    pub lon: f64,
    /// Weighted spread of contributing AP positions around the centroid,
    /// clamped to [10, 1000] m. A rough dispersion estimate, NOT a calibrated
    /// 68%/95% confidence radius (contrast W3C Geolocation / Google Maps
    /// `accuracy`). When `fallback` is true this is the 1000 m sentinel, not a
    /// measured spread — read `fallback` before trusting the magnitude.
    /// The single-surviving-AP path returns a fixed `100.0` (a nominal
    /// single-source estimate, likewise not a computed spread).
    pub accuracy_m: f64,
    /// True iff this position is the antipodal-cancellation fallback
    /// (strongest-AP, the centroid was ambiguous) — `accuracy_m == 1000` here
    /// is a sentinel, NOT a measured spread. On every other path this is
    /// false and `accuracy_m` is the weighted spread of contributing AP
    /// positions around the centroid, clamped to [10, 1000] m.
    pub fallback: bool,
}

/// Physical clamp window for Wi-Fi signal strength (dBm) before it enters the
/// vote-weight exponent. Real RSSI sits in roughly [-100, -30] dBm; we widen
/// to [-120, 0] to tolerate driver quirks while still bounding the exponent.
/// Without this, a bogus value (the iw parser's saturating cast can emit up to
/// i32::MAX) makes 10^(dbm/20) overflow to +inf, and the inf/inf normalization
/// then yields NaN lat/lon returned as a "successful" Position. task-0186.
const DBM_MIN: f64 = -120.0;
const DBM_MAX: f64 = 0.0;

/// dBm assigned to a signal-less AP only when NO AP in the set has a signal
/// (so every AP is equal and the exact value is irrelevant). Mid-range Wi-Fi.
/// When at least one AP has a signal, `missing_signal_dbm` uses the median of
/// those instead. task-0186.
const NOMINAL_MISSING_DBM: f64 = -75.0;

/// Vote weight for one AP in the weighted centroid.
///
/// `10^(dbm/20)` is proportional to 1/distance under the free-space path-loss
/// model, so a stronger (closer-to-0) signal pulls the centroid harder. `dbm`
/// is clamped to `[DBM_MIN, DBM_MAX]` first, so any `i32` signal yields a
/// finite weight in `[1e-6, 1]` and can never overflow the exponent to +inf.
/// A missing signal uses `fallback_dbm` (see `missing_signal_dbm`) rather than
/// a fixed constant, so a signal-less AP cannot dominate a set of weaker
/// measured APs. task-0186.
///
/// task-0126: the parameter is a typed [`Rssi`] rather than a bare `i32`, so
/// the caller cannot accidentally pass some other integer. The math is
/// UNCHANGED — `10^(clamp(dbm)/20)` on `rssi.dbm()` — and therefore returns
/// bit-for-bit the same weight as before for any given input. The measurement
/// source (iw vs nmcli) is deliberately NOT consulted here: vote weight stays
/// source-agnostic; provenance is tracked on `PositionedAp.rssi_source` for a
/// possible future down-weighting policy, not applied today.
fn vote_weight(rssi: Option<Rssi>, fallback_dbm: f64) -> f64 {
    let dbm = rssi.map(|r| r.dbm() as f64).unwrap_or(fallback_dbm);
    10f64.powf(dbm.clamp(DBM_MIN, DBM_MAX) / 20.0)
}

/// The dBm assigned to APs with no measured signal: the median of the measured
/// dBm in the same set. Median (not mean/max) guarantees a signal-less AP
/// never outweighs the strongest measured AP — its resolved dBm sits at the
/// middle of the observed distribution, and `median <= max` is preserved by
/// the monotonic clamp in `vote_weight`. A fixed nominal (e.g. -75 dBm) would
/// NOT satisfy this: in a set of only weak APs (all < -75) it would outweigh
/// every real one. When no AP has a signal, all are equal regardless, so we
/// return `NOMINAL_MISSING_DBM`. task-0186.
fn missing_signal_dbm(aps: &[PositionedAp]) -> f64 {
    let present: Vec<f64> = aps
        .iter()
        .filter_map(|a| a.signal_dbm.map(|d| d as f64))
        .collect();
    if present.is_empty() {
        NOMINAL_MISSING_DBM
    } else {
        median(&present)
    }
}

/// task-0227: final fail-closed gate on a computed trilateration result.
///
/// With the non-finite input filter at `trilaterate`'s entry this should
/// never fire, but routing every `Ok` return through it makes the
/// "trilaterate never yields a non-finite Position" invariant hold
/// *structurally* for all return paths — including any future one — rather
/// than only in the `cfg(test)` asserts. A non-finite Position is a bug, not
/// a location, so we surface it as an `Err` instead of handing a caller
/// `(NaN, NaN)` that looks like a successful fix.
fn finite_position(pos: Position) -> Result<Position> {
    if pos.lat.is_finite() && pos.lon.is_finite() && pos.accuracy_m.is_finite() {
        Ok(pos)
    } else {
        bail!(
            "trilaterate produced a non-finite position (lat={}, lon={}, accuracy_m={})",
            pos.lat,
            pos.lon,
            pos.accuracy_m
        );
    }
}

/// Compute weighted centroid from positioned APs.
///
/// Weight formula: `10^(signal_dbm / 20)` via `vote_weight`. Since signal_dbm
/// is negative (e.g. -40), stronger signals (closer to 0) yield larger weights.
/// E.g. -40 dBm -> weight 0.01, -80 dBm -> weight 0.0001, so the stronger AP
/// gets 100x more influence. dBm is clamped to [-120, 0] first, so an
/// out-of-range value cannot overflow the weight to +inf (which would poison
/// the normalized centroid to NaN). An AP with no signal is given the median
/// of the measured dBm in the set (`missing_signal_dbm`) so it never dominates
/// — the old `None => 1.0` equalled a 0 dBm signal, 100x-10000x a real AP.
pub fn trilaterate(aps: &[PositionedAp]) -> Result<Position> {
    if aps.is_empty() {
        bail!("no positioned APs for trilateration");
    }

    // task-0227: fail closed on non-finite coordinates BEFORE any math.
    // A NaN/±Inf lat or lon (e.g. read from a corrupt SQLite REAL column)
    // poisons to_unit_vec into a NaN vector; the `mag > 0` guards below are
    // false-on-NaN, so from_unit_vec would return (NaN, NaN) as an
    // Ok(Position). Dropping such APs here also makes the single-AP fast
    // path and the strongest-AP fallback safe, since they can only echo an
    // input coordinate that is now guaranteed finite. This is defense in
    // depth behind the server-side cache-read filter (server.rs); direct
    // callers (tests, future consumers) get the same guarantee.
    let finite_fixed: Vec<PositionedAp> = aps
        .iter()
        .copied()
        .filter(|ap| !ap.is_mobile && ap.lat.is_finite() && ap.lon.is_finite())
        .collect();
    if finite_fixed.is_empty() {
        bail!(
            "no fixed, finite positioned APs for trilateration among {} inputs",
            aps.len()
        );
    }
    let aps: &[PositionedAp] = &finite_fixed;

    // Filter outliers before computing centroid
    let filtered = filter_outliers(aps);
    if filtered.is_empty() {
        bail!("no fixed positioned APs survived outlier filtering");
    }
    let aps = filtered.as_slice();

    if aps.len() == 1 {
        return finite_position(Position {
            lat: aps[0].lat,
            lon: aps[0].lon,
            accuracy_m: 100.0,
            fallback: false,
        });
    }

    // Observability only (task-0126): surface how many contributing APs carry
    // authoritative iw dBm vs nmcli's biased linearization. A fix dominated by
    // nmcli-sourced APs has systematically skewed weights; logging the mix lets
    // an operator see that WITHOUT changing any weight — the vote-weight math
    // remains source-agnostic. This also anchors `rssi_source` as genuinely
    // read, not a dead field.
    let nmcli_sourced = aps
        .iter()
        .filter(|ap| ap.rssi_source == RssiSource::NmcliPercentLinearized)
        .count();
    if nmcli_sourced > 0 {
        tracing::debug!(
            target: "whereamid::trilaterate",
            n = aps.len(),
            nmcli_linearized = nmcli_sourced,
            "trilaterating with nmcli-linearized (biased) signal sources present; weights are not source-adjusted"
        );
    }

    // Weighted spherical mean: sum(w_i * v_i) on the unit sphere, then normalize.
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut sz = 0.0;
    let mut sum_weight = 0.0;

    let fallback_dbm = missing_signal_dbm(aps);
    for ap in aps {
        // `from_iw_dbm` is the identity re-wrap of the already-resolved dBm;
        // vote weight is source-agnostic so this changes no numeric result
        // (task-0126). True provenance lives on `ap.rssi_source`.
        let weight = vote_weight(ap.signal_dbm.map(Rssi::from_iw_dbm), fallback_dbm);
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
        return finite_position(Position {
            lat: strongest.lat,
            lon: strongest.lon,
            // accuracy_m is an out-of-band sentinel here, not a spread: the
            // `fallback` flag is the in-protocol signal that the centroid was
            // ambiguous. See the `fallback` doc-comment on `Position`.
            accuracy_m: 1000.0,
            fallback: true,
        });
    }
    let (lat, lon) = from_unit_vec(sx, sy, sz);

    // Estimate accuracy from the weighted spread of input positions
    let accuracy_m = estimate_accuracy(aps, lat, lon);

    finite_position(Position {
        lat,
        lon,
        accuracy_m,
        fallback: false,
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

    let fallback_dbm = missing_signal_dbm(aps);
    for ap in aps {
        let weight = vote_weight(ap.signal_dbm.map(Rssi::from_iw_dbm), fallback_dbm);
        let dist = haversine_m(center_lat, center_lon, ap.lat, ap.lon);
        sum_dist += weight * dist;
        sum_weight += weight;
    }

    let avg_spread = sum_dist / sum_weight;

    // Accuracy is at least 10m, and grows with AP spread
    // The spread itself is a decent proxy for uncertainty
    avg_spread.clamp(10.0, 1000.0)
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
            rssi_source: crate::rssi::RssiSource::IwDbm,
            is_mobile: false,
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
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.8580,
                lon: 2.3520,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
        ];
        let pos = trilaterate(&aps).unwrap();
        assert!((pos.lat - 48.8575).abs() < 1e-4);
        assert!((pos.lon - 2.3515).abs() < 1e-4);
        // A legitimate multi-AP fix is never the antipodal fallback, even
        // though its clamped accuracy could in principle reach 1000 m.
        assert!(!pos.fallback, "legit multi-AP fix must not set fallback");
    }

    #[test]
    fn test_stronger_signal_more_weight() {
        let aps = vec![
            PositionedAp {
                lat: 55.0,
                lon: 12.0,
                signal_dbm: Some(-40), // very strong
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 56.0,
                lon: 13.0,
                signal_dbm: Some(-80), // weak
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
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

    /// task-0126 AC#5: a stronger (closer-to-0) dBm yields a larger vote
    /// weight. Uses the typed `Rssi` constructor; the fallback is irrelevant
    /// because both inputs are `Some`. Guards the `10^(dbm/20)` monotonicity
    /// through the newtype refactor.
    #[test]
    fn stronger_rssi_has_greater_vote_weight() {
        let fb = -75.0;
        let strong = vote_weight(Some(Rssi::from_iw_dbm(-60)), fb);
        let weak = vote_weight(Some(Rssi::from_iw_dbm(-80)), fb);
        assert!(
            strong > weak,
            "stronger -60 dBm weight {strong} must exceed weaker -80 dBm weight {weak}"
        );
    }

    #[test]
    fn mobile_aps_are_removed_before_filtering_and_trilateration() {
        let aps = vec![
            PositionedAp {
                lat: 55.6760,
                lon: 12.5680,
                signal_dbm: Some(-60),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.6761,
                lon: 12.5682,
                signal_dbm: Some(-60),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.6759,
                lon: 12.5681,
                signal_dbm: Some(-60),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            // A coherent train cluster kilometers away. It must be removed
            // before drop_isolated so the two mobile APs cannot save each
            // other as neighbors.
            PositionedAp {
                lat: 55.7390,
                lon: 12.5500,
                signal_dbm: Some(-30),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: true,
            },
            PositionedAp {
                lat: 55.7391,
                lon: 12.5501,
                signal_dbm: Some(-30),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: true,
            },
        ];

        let filtered = filter_outliers(&aps);
        assert_eq!(filtered.len(), 3);
        assert!(filtered.iter().all(|ap| !ap.is_mobile));

        let expected = trilaterate(&aps[..3]).unwrap();
        let actual = trilaterate(&aps).unwrap();
        assert!((actual.lat - expected.lat).abs() < 1e-12);
        assert!((actual.lon - expected.lon).abs() < 1e-12);
        assert!((actual.accuracy_m - expected.accuracy_m).abs() < 1e-9);
    }

    #[test]
    fn mobile_filter_applies_before_small_input_fast_path() {
        let fixed = PositionedAp {
            lat: 55.6761,
            lon: 12.5683,
            signal_dbm: Some(-80),
            rssi_source: crate::rssi::RssiSource::IwDbm,
            is_mobile: false,
        };
        let aps = vec![
            fixed,
            PositionedAp {
                lat: 10.0,
                lon: 20.0,
                signal_dbm: Some(-20),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: true,
            },
            PositionedAp {
                lat: 10.1,
                lon: 20.1,
                signal_dbm: Some(-20),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: true,
            },
        ];

        assert_eq!(filter_outliers(&aps), vec![fixed]);
        let result = trilaterate(&aps).unwrap();
        assert_eq!(result.lat, fixed.lat);
        assert_eq!(result.lon, fixed.lon);
    }

    #[test]
    fn all_mobile_input_has_no_position() {
        let aps = vec![
            PositionedAp {
                lat: 55.0,
                lon: 12.0,
                signal_dbm: Some(-40),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: true,
            },
            PositionedAp {
                lat: 55.1,
                lon: 12.1,
                signal_dbm: Some(-50),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: true,
            },
        ];
        assert!(filter_outliers(&aps).is_empty());
        assert!(trilaterate(&aps).is_err());
    }

    /// task-0186 AC1: a signal-less AP must never outweigh the strongest
    /// measured AP — including a set where every measured AP is WEAKER than a
    /// fixed nominal like -75 dBm (the case a constant fallback breaks). The
    /// median-of-present fallback keeps the None weight <= the max weight.
    #[test]
    fn missing_signal_never_outweighs_strongest_measured() {
        let aps = [
            PositionedAp {
                lat: 0.0,
                lon: 0.0,
                signal_dbm: Some(-80),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.0,
                lon: 0.0,
                signal_dbm: Some(-90),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.0,
                lon: 0.0,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
        ];
        let fb = missing_signal_dbm(&aps);
        let none_w = vote_weight(None, fb);
        let strongest_w = vote_weight(Some(Rssi::from_iw_dbm(-80)), fb); // strongest measured
        assert!(
            none_w <= strongest_w,
            "None weight {none_w} must be <= strongest measured weight {strongest_w}"
        );
        // median(-80, -90) = -85 < -80, so strictly less here.
        assert!(none_w < strongest_w, "expected strictly less for this set");
        // Sanity: a fixed -75 nominal WOULD have violated AC1 for this set.
        let fixed_nominal_w = 10f64.powf(-75.0 / 20.0);
        assert!(
            fixed_nominal_w > strongest_w,
            "a fixed -75 nominal would wrongly outweigh the -80 AP"
        );
    }

    /// task-0186 AC2: any i32 signal_dbm must yield a finite lat/lon/accuracy.
    /// Before the clamp, a huge dbm (the iw parser's saturating cast can emit
    /// i32::MAX) overflowed 10^(dbm/20) to +inf and the normalization produced
    /// NaN lat/lon returned as a "successful" Position.
    #[test]
    fn extreme_signal_dbm_yields_finite_position() {
        let aps = vec![
            PositionedAp {
                lat: 55.700,
                lon: 12.580,
                signal_dbm: Some(i32::MAX),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.701,
                lon: 12.581,
                signal_dbm: Some(i32::MIN),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.700,
                lon: 12.585,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
        ];
        let pos = trilaterate(&aps).unwrap();
        assert!(pos.lat.is_finite(), "lat must be finite, got {}", pos.lat);
        assert!(pos.lon.is_finite(), "lon must be finite, got {}", pos.lon);
        assert!(
            pos.accuracy_m.is_finite(),
            "accuracy must be finite, got {}",
            pos.accuracy_m
        );
    }

    /// task-0227 AC#3: a non-finite coordinate must be EXCLUDED, never
    /// returned as a fix. A lone NaN/Inf AP has nothing positionable left
    /// after the entry filter, so trilaterate errors exactly like the
    /// empty-input case — it must not hand back `Ok((NaN, NaN))`.
    #[test]
    fn lone_non_finite_ap_errors_not_returned() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let lat_bad = vec![PositionedAp {
                lat: bad,
                lon: 12.5,
                signal_dbm: Some(-60),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            }];
            assert!(
                trilaterate(&lat_bad).is_err(),
                "lone AP with lat={bad} must error, not return a position"
            );
            let lon_bad = vec![PositionedAp {
                lat: 55.6,
                lon: bad,
                signal_dbm: None,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            }];
            assert!(
                trilaterate(&lon_bad).is_err(),
                "lone AP with lon={bad} must error, not return a position"
            );
        }
    }

    /// task-0227 AC#3: a non-finite AP mixed into an otherwise-finite cluster
    /// must not contribute — the result stays finite and sits inside the
    /// finite cluster (the poisoned AP, despite a strong signal, pulls
    /// nothing). Exercises the centroid path with the poisoned AP dropped.
    #[test]
    fn non_finite_ap_excluded_from_cluster() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let aps = vec![
                // Poisoned AP with a strong signal that WOULD dominate the
                // weighted centroid if it were not filtered out first.
                PositionedAp {
                    lat: bad,
                    lon: bad,
                    signal_dbm: Some(-30),
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 55.700,
                    lon: 12.580,
                    signal_dbm: Some(-60),
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 55.701,
                    lon: 12.581,
                    signal_dbm: Some(-65),
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    is_mobile: false,
                },
            ];
            let pos = trilaterate(&aps).unwrap();
            assert!(
                pos.lat.is_finite() && pos.lon.is_finite() && pos.accuracy_m.is_finite(),
                "result must be finite with a {bad} AP present, got ({}, {}, {})",
                pos.lat,
                pos.lon,
                pos.accuracy_m
            );
            // The centroid must land in the finite cluster, not near the
            // poisoned coordinate — proof the bad AP contributed nothing.
            assert!(
                (pos.lat - 55.7005).abs() < 0.1 && (pos.lon - 12.5805).abs() < 0.1,
                "centroid drifted from the finite cluster with a {bad} AP: ({}, {})",
                pos.lat,
                pos.lon
            );
        }
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
                lat: 48.856,
                lon: 2.352,
                signal_dbm: Some(-75),
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
                lat: -12.894,
                lon: -38.292,
                signal_dbm: Some(-60),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            }, // distant outlier
        ];
        let pos = trilaterate(&aps).unwrap();
        assert!(pos.lat > 48.0, "lat should be in cluster, got {}", pos.lat);
        assert!(pos.lon > 2.0, "lon should be in cluster, got {}", pos.lon);
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
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.0,
                lon: -179.0,
                signal_dbm: Some(-50),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
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
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 89.0,
                lon: 180.0,
                signal_dbm: Some(-50),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
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
    fn antipodal_aps_unequal_weight_resolve_via_weighted_mean_not_fallback() {
        // Two APs on opposite sides of the planet with UNEQUAL signal. The
        // vector sum is (w1-w2)*v, whose magnitude 0.0099 dwarfs the guard
        // threshold sum_weight*1e-6 (~1e-8), so the antipodal-cancellation
        // guard does NOT fire — this input resolves through the ordinary
        // weighted-mean path. The mean coincidentally collapses onto the
        // stronger AP (0,0) and the wide spread clamps accuracy to 1000. The
        // guard fires only when the two weights are (near-)equal; see
        // `antipodal_equal_weight_triggers_fallback`. So `fallback` must be
        // false here even though accuracy_m == 1000 — exactly the ambiguity
        // this task removes from the wire.
        let aps = vec![
            PositionedAp {
                lat: 0.0,
                lon: 0.0,
                signal_dbm: Some(-40),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.0,
                lon: 180.0,
                signal_dbm: Some(-80),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
        ];
        let pos = trilaterate(&aps).unwrap();
        assert!(
            pos.lat.abs() < 1e-6 && pos.lon.abs() < 1e-6,
            "weighted mean should collapse onto the stronger AP at (0,0), got ({},{})",
            pos.lat,
            pos.lon
        );
        assert_eq!(
            pos.accuracy_m, 1000.0,
            "wide spread must clamp to the 1000 m ceiling, got {}",
            pos.accuracy_m
        );
        assert!(
            !pos.fallback,
            "unequal-weight antipodal input does not trip the guard; fallback must be false \
             even though accuracy_m clamped to 1000"
        );
    }

    #[test]
    fn antipodal_equal_weight_triggers_fallback() {
        // AC#5: EQUAL-weight antipodal APs are the only case that fires the
        // cancellation guard (the vector sum is exactly zero). The result is
        // the strongest-AP fallback with the out-of-band `fallback` flag set
        // and accuracy_m at the 1000 m sentinel — distinct on the wire from a
        // legitimately-wide fix that clamps to 1000 with fallback=false.
        let aps = vec![
            PositionedAp {
                lat: 0.0,
                lon: 0.0,
                signal_dbm: Some(-50),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.0,
                lon: 180.0,
                signal_dbm: Some(-50),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
        ];
        let pos = trilaterate(&aps).unwrap();
        assert!(
            pos.fallback,
            "equal-weight antipodal input must set fallback=true"
        );
        assert_eq!(
            pos.accuracy_m, 1000.0,
            "fallback sentinel accuracy must be exactly 1000, got {}",
            pos.accuracy_m
        );
        // The reported point is one of the two (tied) inputs.
        let on_input = (pos.lat.abs() < 1e-6)
            && (pos.lon.abs() < 1e-6 || (pos.lon.abs() - 180.0).abs() < 1e-6);
        assert!(
            on_input,
            "fallback must echo one of the antipodal inputs, got ({},{})",
            pos.lat, pos.lon
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
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.70709,
                lon: 12.58565,
                signal_dbm: Some(-72),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.70735,
                lon: 12.58569,
                signal_dbm: Some(-73),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.70714,
                lon: 12.58544,
                signal_dbm: Some(-73),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.70713,
                lon: 12.58544,
                signal_dbm: Some(-73),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.70662,
                lon: 12.58570,
                signal_dbm: Some(-77),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
            },
            // 1 Brazilian outlier — STRONGEST signal, so without filtering
            // it would dominate the weighted centroid.
            PositionedAp {
                lat: -12.89422,
                lon: -38.29226,
                signal_dbm: Some(-49),
                rssi_source: crate::rssi::RssiSource::IwDbm,
                is_mobile: false,
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
}
