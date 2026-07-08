//! Weighted centroid trilateration from AP positions and signal strengths.
//!
//! Centroid math operates on the **unit sphere**, not on raw lat/lon. This
//! avoids the antimeridian discontinuity (an arithmetic mean of lon=+179 and
//! lon=-179 yields lon=0, which is on the wrong side of the planet) and the
//! polar singularity (longitude is degenerate near the poles). Each (lat, lon)
//! is mapped to a unit 3-vector, vectors are weighted-summed and normalized,
//! then mapped back to (lat, lon) via atan2/asin.

use anyhow::{bail, Result};
use std::fmt;

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

/// Input for trilateration: a positioned AP with optional signal.
///
/// `Copy` (task-0086): every field is itself `Copy` (f64, f64, Option<i32>, bool),
/// so cloning is already a bitwise memcpy. Deriving `Copy` removes the
/// ergonomic noise of explicit `.clone()` calls. Caveat: this is a soft
/// promise to consumers — if a future field is non-Copy (e.g. an owning
/// SSID `String` or a `Source` enum), reverting forces every by-value call
/// site to clone or take a reference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedAp {
    pub lat: f64,
    pub lon: f64,
    pub signal_dbm: Option<i32>,
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
fn median(values: &[f64]) -> f64 {
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
fn vote_weight(signal_dbm: Option<i32>, fallback_dbm: f64) -> f64 {
    let dbm = signal_dbm.map(|d| d as f64).unwrap_or(fallback_dbm);
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

    // Weighted spherical mean: sum(w_i * v_i) on the unit sphere, then normalize.
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut sz = 0.0;
    let mut sum_weight = 0.0;

    let fallback_dbm = missing_signal_dbm(aps);
    for ap in aps {
        let weight = vote_weight(ap.signal_dbm, fallback_dbm);
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
        let weight = vote_weight(ap.signal_dbm, fallback_dbm);
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

    #[test]
    fn test_single_ap() {
        let aps = vec![PositionedAp {
            lat: 55.6684,
            lon: 12.5541,
            signal_dbm: Some(-65),
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
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.8580,
                lon: 2.3520,
                signal_dbm: None,
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
                is_mobile: false,
            },
            PositionedAp {
                lat: 56.0,
                lon: 13.0,
                signal_dbm: Some(-80), // weak
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

    #[test]
    fn mobile_aps_are_removed_before_filtering_and_trilateration() {
        let aps = vec![
            PositionedAp {
                lat: 55.6760,
                lon: 12.5680,
                signal_dbm: Some(-60),
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.6761,
                lon: 12.5682,
                signal_dbm: Some(-60),
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.6759,
                lon: 12.5681,
                signal_dbm: Some(-60),
                is_mobile: false,
            },
            // A coherent train cluster kilometers away. It must be removed
            // before drop_isolated so the two mobile APs cannot save each
            // other as neighbors.
            PositionedAp {
                lat: 55.7390,
                lon: 12.5500,
                signal_dbm: Some(-30),
                is_mobile: true,
            },
            PositionedAp {
                lat: 55.7391,
                lon: 12.5501,
                signal_dbm: Some(-30),
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
            is_mobile: false,
        };
        let aps = vec![
            fixed,
            PositionedAp {
                lat: 10.0,
                lon: 20.0,
                signal_dbm: Some(-20),
                is_mobile: true,
            },
            PositionedAp {
                lat: 10.1,
                lon: 20.1,
                signal_dbm: Some(-20),
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
                is_mobile: true,
            },
            PositionedAp {
                lat: 55.1,
                lon: 12.1,
                signal_dbm: Some(-50),
                is_mobile: true,
            },
        ];
        assert!(filter_outliers(&aps).is_empty());
        assert!(trilaterate(&aps).is_err());
    }

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
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.0,
                lon: 0.0,
                signal_dbm: Some(-90),
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.0,
                lon: 0.0,
                signal_dbm: None,
                is_mobile: false,
            },
        ];
        let fb = missing_signal_dbm(&aps);
        let none_w = vote_weight(None, fb);
        let strongest_w = vote_weight(Some(-80), fb); // strongest measured
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
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.701,
                lon: 12.581,
                signal_dbm: Some(i32::MIN),
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.700,
                lon: 12.585,
                signal_dbm: None,
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
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 55.700,
                    lon: 12.580,
                    signal_dbm: Some(-60),
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 55.701,
                    lon: 12.581,
                    signal_dbm: Some(-65),
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
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.857,
                lon: 2.352,
                signal_dbm: Some(-65),
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.858,
                lon: 2.351,
                signal_dbm: Some(-70),
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.856,
                lon: 2.352,
                signal_dbm: Some(-75),
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.857,
                lon: 2.350,
                signal_dbm: Some(-68),
                is_mobile: false,
            },
            PositionedAp {
                lat: -12.894,
                lon: -38.292,
                signal_dbm: Some(-60),
                is_mobile: false,
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
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.857,
                lon: 2.352,
                signal_dbm: Some(-65),
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.858,
                lon: 2.351,
                signal_dbm: Some(-70),
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.857,
                lon: 2.350,
                signal_dbm: Some(-68),
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.875,
                lon: 2.325,
                signal_dbm: Some(-62),
                is_mobile: false,
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
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.0,
                lon: -179.0,
                signal_dbm: Some(-50),
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
    fn antimeridian_filter_outliers_does_not_mistake_dateline_cluster() {
        // Three APs all near the antimeridian — none should be rejected as outliers.
        let aps = vec![
            PositionedAp {
                lat: 0.0,
                lon: 179.99,
                signal_dbm: Some(-60),
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.0,
                lon: -179.99,
                signal_dbm: Some(-60),
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.001,
                lon: 180.0_f64.copysign(1.0),
                signal_dbm: Some(-60),
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
                is_mobile: false,
            },
            PositionedAp {
                lat: 89.0,
                lon: 180.0,
                signal_dbm: Some(-50),
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
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.0,
                lon: 180.0,
                signal_dbm: Some(-80),
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
                is_mobile: false,
            },
            PositionedAp {
                lat: 0.0,
                lon: 180.0,
                signal_dbm: Some(-50),
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

    /// task-0083: pin stage-1 behavior directly so a refactor of
    /// `filter_outliers` can't silently lose the Brazil-catcher property.
    #[test]
    fn drop_isolated_drops_lone_outlier() {
        let aps = [
            PositionedAp {
                lat: 55.707,
                lon: 12.585,
                signal_dbm: Some(-70),
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.708,
                lon: 12.585,
                signal_dbm: Some(-72),
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.706,
                lon: 12.586,
                signal_dbm: Some(-74),
                is_mobile: false,
            },
            // Brazil — no neighbor within NEIGHBOR_RADIUS_M.
            PositionedAp {
                lat: -12.894,
                lon: -38.292,
                signal_dbm: Some(-49),
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
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.5,
                lon: 12.5,
                signal_dbm: None,
                is_mobile: false,
            },
            PositionedAp {
                lat: 56.0,
                lon: 13.0,
                signal_dbm: None,
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
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.701,
                lon: 12.581,
                signal_dbm: None,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.699,
                lon: 12.579,
                signal_dbm: None,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.700,
                lon: 12.582,
                signal_dbm: None,
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.702,
                lon: 12.580,
                signal_dbm: None,
                is_mobile: false,
            },
            // 4 outliers on 4 different continents
            PositionedAp {
                lat: -33.87,
                lon: 151.21,
                signal_dbm: None,
                is_mobile: false,
            }, // Sydney
            PositionedAp {
                lat: 35.68,
                lon: 139.69,
                signal_dbm: None,
                is_mobile: false,
            }, // Tokyo
            PositionedAp {
                lat: -12.89,
                lon: -38.29,
                signal_dbm: None,
                is_mobile: false,
            }, // Salvador
            PositionedAp {
                lat: 40.71,
                lon: -74.00,
                signal_dbm: None,
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
        use std::sync::{Arc, Mutex};
        use tracing::{
            field::{Field, Visit},
            span, subscriber, Event, Metadata, Subscriber,
        };

        // Minimal Subscriber that records (level, target, message) for
        // every event. We use this rather than a full
        // tracing_subscriber::Registry to keep the test free of extra
        // moving parts.
        #[derive(Default, Clone)]
        struct Captured(Arc<Mutex<Vec<(tracing::Level, String, String)>>>);
        struct MsgVisitor(String);
        impl Visit for MsgVisitor {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                }
            }
            fn record_str(&mut self, field: &Field, value: &str) {
                if field.name() == "message" {
                    self.0 = value.to_string();
                }
            }
        }
        impl Subscriber for Captured {
            fn enabled(&self, _md: &Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _attrs: &span::Attributes<'_>) -> span::Id {
                span::Id::from_u64(1)
            }
            fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
            fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
            fn event(&self, event: &Event<'_>) {
                let mut v = MsgVisitor(String::new());
                event.record(&mut v);
                let md = event.metadata();
                self.0
                    .lock()
                    .unwrap()
                    .push((*md.level(), md.target().to_string(), v.0));
            }
            fn enter(&self, _: &span::Id) {}
            fn exit(&self, _: &span::Id) {}
        }

        crate::test_support::install_permissive_global_test_subscriber(); // task-0251
        let captured = Captured::default();
        let cloned = captured.clone();

        subscriber::with_default(captured, || {
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
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 0.0,
                    lon: 0.0001,
                    signal_dbm: Some(-60),
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 0.0,
                    lon: 180.0,
                    signal_dbm: Some(-60),
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
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 0.0,
                    lon: 0.0,
                    signal_dbm: Some(-60),
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 0.0,
                    lon: 180.0,
                    signal_dbm: Some(-60),
                    is_mobile: false,
                },
                PositionedAp {
                    lat: 0.0,
                    lon: 180.0,
                    signal_dbm: Some(-60),
                    is_mobile: false,
                },
            ];
            let _ = filter_outliers(&aps_deg);
        });

        // Assert: at least one event was emitted with the expected
        // target. We don't pin the exact message text — that's free to
        // evolve — but we do require it came from
        // whereamid::trilaterate.
        let events = cloned.0.lock().unwrap().clone();
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
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.70709,
                lon: 12.58565,
                signal_dbm: Some(-72),
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.70735,
                lon: 12.58569,
                signal_dbm: Some(-73),
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.70714,
                lon: 12.58544,
                signal_dbm: Some(-73),
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.70713,
                lon: 12.58544,
                signal_dbm: Some(-73),
                is_mobile: false,
            },
            PositionedAp {
                lat: 55.70662,
                lon: 12.58570,
                signal_dbm: Some(-77),
                is_mobile: false,
            },
            // 1 Brazilian outlier — STRONGEST signal, so without filtering
            // it would dominate the weighted centroid.
            PositionedAp {
                lat: -12.89422,
                lon: -38.29226,
                signal_dbm: Some(-49),
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

    #[test]
    fn test_no_outliers_all_close() {
        // All APs within 200m — none should be rejected
        let aps = vec![
            PositionedAp {
                lat: 48.8570,
                lon: 2.3510,
                signal_dbm: Some(-60),
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.8571,
                lon: 2.3515,
                signal_dbm: Some(-65),
                is_mobile: false,
            },
            PositionedAp {
                lat: 48.8569,
                lon: 2.3512,
                signal_dbm: Some(-70),
                is_mobile: false,
            },
        ];
        let filtered = filter_outliers(&aps);
        assert_eq!(filtered.len(), 3);
    }
}
