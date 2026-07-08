//! Location history: timeseries of resolved fixes grouped into stay-point
//! segments.
//!
//! A "fix" is one successful locate response (lat/lon/accuracy + timestamp).
//! Storing every fix as a raw row would let users query "where was I N
//! days ago", but the raw stream is dense and visually noisy. Segmentation
//! collapses contiguous fixes within `dist_threshold_m` of each other into
//! a single `Segment` (start, end, centroid, accuracy, count). Segments
//! whose duration is below `min_duration_secs` are dropped — a 30-second
//! detour past a coffee shop is not a "place".

use chrono::DateTime;
use serde::Serialize;

use crate::db::FixRow;

/// A grouped stay-point: a run of fixes within `dist_threshold_m` of each
/// other for at least `min_duration_secs`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Segment {
    /// First fix in the segment (RFC3339).
    pub start_rfc3339: String,
    /// Last fix in the segment (RFC3339).
    pub end_rfc3339: String,
    /// Duration in seconds.
    pub duration_secs: i64,
    /// Centroid latitude (geometric median of fix latitudes — task-0084).
    pub centroid_lat: f64,
    /// Centroid longitude (geometric median of fix longitudes — task-0084).
    pub centroid_lon: f64,
    /// Mean accuracy_m of fixes in the segment.
    pub mean_accuracy_m: f64,
    /// Number of fixes aggregated.
    pub n_fixes: usize,
}

/// Result of cancellation-aware history segmentation.
///
/// `Cancelled` is explicit so a caller cannot accidentally serve a partial
/// list of segments as a successful response.
#[derive(Debug, PartialEq)]
pub(crate) enum SegmentFixesOutcome {
    Complete(Vec<Segment>),
    Cancelled,
}

/// The cheap membership center for the segment currently being built.
///
/// Adding a point is O(1): latitude/longitude is projected to a unit vector
/// and accumulated. Converting the sum back to latitude/longitude handles
/// both the antimeridian and poles without a planar-longitude discontinuity.
/// When a genuinely degenerate vector sum cancels, use the first fix as the
/// documented stable fallback.
struct RunningSphericalMean {
    x: f64,
    y: f64,
    z: f64,
    first_fix: Option<(f64, f64)>,
}

impl RunningSphericalMean {
    fn new(fix: &FixRow) -> Self {
        let first_fix = finite_coord(fix);
        let mut mean = Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            first_fix,
        };
        mean.add(fix);
        mean
    }

    fn add(&mut self, fix: &FixRow) {
        let Some((lat, lon)) = finite_coord(fix) else {
            return;
        };
        let (x, y, z) = crate::trilaterate::to_unit_vec(lat, lon);
        self.x += x;
        self.y += y;
        self.z += z;
    }

    fn center(&self) -> Option<(f64, f64)> {
        let magnitude = (self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if magnitude.is_finite() && magnitude >= 1e-9 {
            let point = crate::trilaterate::from_unit_vec(self.x, self.y, self.z);
            if point.0.is_finite() && point.1.is_finite() {
                return Some(point);
            }
        }
        self.first_fix
    }
}

fn finite_coord(fix: &FixRow) -> Option<(f64, f64)> {
    (fix.lat.is_finite() && fix.lon.is_finite()).then_some((fix.lat, fix.lon))
}

#[derive(Debug)]
enum MedianComputation {
    Point((f64, f64)),
    Degenerate,
    Cancelled,
}

trait MedianComputer {
    fn compute(
        &mut self,
        coords: &[(f64, f64)],
        is_cancelled: &mut dyn FnMut() -> bool,
    ) -> MedianComputation;
}

impl<F> MedianComputer for F
where
    F: FnMut(&[(f64, f64)], &mut dyn FnMut() -> bool) -> MedianComputation,
{
    fn compute(
        &mut self,
        coords: &[(f64, f64)],
        is_cancelled: &mut dyn FnMut() -> bool,
    ) -> MedianComputation {
        self(coords, is_cancelled)
    }
}

fn compute_geometric_median(
    coords: &[(f64, f64)],
    is_cancelled: &mut dyn FnMut() -> bool,
) -> MedianComputation {
    use crate::trilaterate::{
        geometric_median_detailed_cancellable, CancellableGeoMedianResult, GeoMedianResult,
    };

    // NOTE: seg_id is not available at this layer. `compute_geometric_median`
    // is an injected `MedianComputer` fn that receives only `(coords,
    // is_cancelled)`; threading the segment id through the trait and its
    // test double would be invasive and out of proportion for a log line
    // (see task-0141). We log the iterate and `coords.len()` instead, which
    // is enough to correlate a cap/fallback with the flushed segment.
    match geometric_median_detailed_cancellable(coords, is_cancelled) {
        CancellableGeoMedianResult::Completed(GeoMedianResult::Converged(point)) => {
            MedianComputation::Point(point)
        }
        CancellableGeoMedianResult::Completed(GeoMedianResult::Capped(point)) => {
            // Weiszfeld hit WEISZFELD_MAX_ITERS without converging. The last
            // iterate is still used best-effort (it is usually close to the
            // true median), but the caller should know the reported segment
            // centroid did not fully converge.
            tracing::debug!(
                ?point,
                coord_count = coords.len(),
                "history segment geometric median hit the Weiszfeld iteration cap \
                 without converging; using the last iterate best-effort"
            );
            MedianComputation::Point(point)
        }
        CancellableGeoMedianResult::Completed(variant @ GeoMedianResult::Antipodal) => {
            tracing::warn!(
                variant = %variant,
                coord_count = coords.len(),
                "history segment geometric median is degenerate (near-antipodal \
                 cluster); segment centroid falls back to a non-median estimate"
            );
            MedianComputation::Degenerate
        }
        CancellableGeoMedianResult::Completed(variant @ GeoMedianResult::Degenerate) => {
            tracing::warn!(
                variant = %variant,
                coord_count = coords.len(),
                "history segment geometric median is degenerate (unusable seed / \
                 invalid coords); segment centroid falls back to a non-median estimate"
            );
            MedianComputation::Degenerate
        }
        CancellableGeoMedianResult::Cancelled => MedianComputation::Cancelled,
    }
}

/// Group consecutive fixes (already in ascending time order) into stay-point
/// segments. A new segment is started when the next fix is more than
/// `dist_threshold_m` from the running cluster centroid. Segments shorter
/// than `min_duration_secs` are discarded.
///
/// The fixes array MUST be sorted ascending by timestamp; the caller (DB
/// layer) does that.
#[cfg(test)]
pub fn segment_fixes(
    fixes: &[FixRow],
    dist_threshold_m: f64,
    min_duration_secs: i64,
) -> Vec<Segment> {
    match segment_fixes_cancellable(fixes, dist_threshold_m, min_duration_secs, || false) {
        SegmentFixesOutcome::Complete(segments) => segments,
        SegmentFixesOutcome::Cancelled => {
            unreachable!("the non-cancellable history wrapper cannot be cancelled")
        }
    }
}

/// Cancellation-aware segmentation path used by TASK-0201's blocking history
/// worker. The async request's Drop guard and daemon shutdown token are folded
/// into this callback so an aborted waiter never yields a partial response.
pub(crate) fn segment_fixes_cancellable<C>(
    fixes: &[FixRow],
    dist_threshold_m: f64,
    min_duration_secs: i64,
    mut is_cancelled: C,
) -> SegmentFixesOutcome
where
    C: FnMut() -> bool,
{
    let mut median = compute_geometric_median;
    segment_fixes_impl(
        fixes,
        dist_threshold_m,
        min_duration_secs,
        &mut is_cancelled,
        &mut median,
    )
}

/// Injectable core: tests supply a counting median implementation to verify
/// total median work structurally, without process-global atomics or races
/// between parallel tests.
fn segment_fixes_impl(
    fixes: &[FixRow],
    dist_threshold_m: f64,
    min_duration_secs: i64,
    is_cancelled: &mut dyn FnMut() -> bool,
    median: &mut impl MedianComputer,
) -> SegmentFixesOutcome {
    if is_cancelled() {
        return SegmentFixesOutcome::Cancelled;
    }
    let Some(first) = fixes.first() else {
        return SegmentFixesOutcome::Complete(Vec::new());
    };

    let mut segments = Vec::new();
    let mut cur: Vec<&FixRow> = vec![first];
    let mut running_center = RunningSphericalMean::new(first);

    for fix in fixes.iter().skip(1) {
        if is_cancelled() {
            return SegmentFixesOutcome::Cancelled;
        }
        // Membership uses the O(1) running spherical mean. The robust
        // geometric median is reserved for the reported centroid at flush,
        // where it runs exactly once for each retained segment.
        let joins = running_center.center().is_some_and(|(lat, lon)| {
            let distance = haversine_m(lat, lon, fix.lat, fix.lon);
            distance.is_finite() && distance <= dist_threshold_m
        });
        if joins {
            cur.push(fix);
            running_center.add(fix);
        } else {
            if flush_segment(&cur, min_duration_secs, &mut segments, is_cancelled, median)
                == FlushOutcome::Cancelled
            {
                return SegmentFixesOutcome::Cancelled;
            }
            cur = vec![fix];
            running_center = RunningSphericalMean::new(fix);
        }
    }
    if flush_segment(&cur, min_duration_secs, &mut segments, is_cancelled, median)
        == FlushOutcome::Cancelled
    {
        return SegmentFixesOutcome::Cancelled;
    }

    SegmentFixesOutcome::Complete(segments)
}

#[derive(Clone, Copy, PartialEq)]
enum FlushOutcome {
    Complete,
    Cancelled,
}

fn flush_segment(
    cur: &[&FixRow],
    min_duration_secs: i64,
    segments: &mut Vec<Segment>,
    is_cancelled: &mut dyn FnMut() -> bool,
    median: &mut impl MedianComputer,
) -> FlushOutcome {
    if is_cancelled() {
        return FlushOutcome::Cancelled;
    }
    let Some(start) = cur.first() else {
        return FlushOutcome::Complete;
    };
    let end = cur
        .last()
        .expect("a segment with a first fix has a last fix");
    // Duration: end - start (ts strings are RFC3339, parseable).
    let duration_secs = match (
        DateTime::parse_from_rfc3339(&start.at_rfc3339),
        DateTime::parse_from_rfc3339(&end.at_rfc3339),
    ) {
        (Ok(s), Ok(e)) => (e - s).num_seconds(),
        _ => 0,
    };
    if duration_secs < min_duration_secs {
        // Drop short segments before marshalling coordinates or invoking the
        // median — they are not reported stay-points.
        return FlushOutcome::Complete;
    }

    let mut coords = Vec::with_capacity(cur.len());
    let mut acc_sum = 0.0;
    for fix in cur {
        if is_cancelled() {
            return FlushOutcome::Cancelled;
        }
        coords.push((fix.lat, fix.lon));
        acc_sum += fix.accuracy_m;
    }

    let first_coord = finite_coord(start);
    let centroid = match median.compute(&coords, is_cancelled) {
        MedianComputation::Point(point) if point.0.is_finite() && point.1.is_finite() => {
            Some(point)
        }
        MedianComputation::Point(_) | MedianComputation::Degenerate => first_coord,
        MedianComputation::Cancelled => return FlushOutcome::Cancelled,
    };
    if is_cancelled() {
        return FlushOutcome::Cancelled;
    }
    let Some((centroid_lat, centroid_lon)) = centroid else {
        // A segment whose first fix is non-finite has no safe documented
        // fallback. Do not emit a NaN centroid into the JSON response.
        return FlushOutcome::Complete;
    };

    segments.push(Segment {
        start_rfc3339: start.at_rfc3339.clone(),
        end_rfc3339: end.at_rfc3339.clone(),
        duration_secs,
        centroid_lat,
        centroid_lon,
        mean_accuracy_m: acc_sum / cur.len() as f64,
        n_fixes: cur.len(),
    });
    FlushOutcome::Complete
}

// task-0081: haversine_m moved to crate::geo. Use it via the path below.
use crate::geo::haversine_m;

// task-0137: parse_range moved to crate::duration_spec (it is a generic
// duration-spec parser, not history-specific). server.rs now calls it via
// crate::duration_spec::parse_range.

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn fix(ts: &str, lat: f64, lon: f64, acc: f64) -> FixRow {
        FixRow {
            id: 0,
            at_rfc3339: ts.to_string(),
            lat,
            lon,
            accuracy_m: acc,
            n_sources: 3,
            quality: None,
        }
    }

    #[test]
    fn segment_empty_input() {
        let segs = segment_fixes(&[], 100.0, 60);
        assert!(segs.is_empty());
    }

    #[test]
    fn segment_single_short_fix_dropped_by_min_duration() {
        // Just one fix → duration 0 → dropped (below min_duration 60s).
        let fixes = vec![fix("2026-05-09T12:00:00+00:00", 55.0, 12.0, 30.0)];
        let segs = segment_fixes(&fixes, 100.0, 60);
        assert!(
            segs.is_empty(),
            "single fix has zero duration; must be dropped"
        );
    }

    #[test]
    fn segment_contiguous_stay_collapses_to_one_segment() {
        // Five fixes within 50m over 10 minutes — one segment.
        let fixes = vec![
            fix("2026-05-09T12:00:00+00:00", 55.6684, 12.5541, 30.0),
            fix("2026-05-09T12:02:30+00:00", 55.6685, 12.5542, 30.0),
            fix("2026-05-09T12:05:00+00:00", 55.6683, 12.5540, 30.0),
            fix("2026-05-09T12:07:30+00:00", 55.6684, 12.5543, 30.0),
            fix("2026-05-09T12:10:00+00:00", 55.6685, 12.5541, 30.0),
        ];
        let segs = segment_fixes(&fixes, 100.0, 60);
        assert_eq!(segs.len(), 1, "expected 1 stay-point segment, got {segs:?}");
        let s = &segs[0];
        assert_eq!(s.n_fixes, 5);
        assert_eq!(s.duration_secs, 600);
        assert!((s.centroid_lat - 55.6684).abs() < 1e-3);
        assert!((s.centroid_lon - 12.5541).abs() < 1e-3);
    }

    #[test]
    fn segment_dispersed_fixes_split_into_multiple_segments() {
        // Two clusters separated by ~5 km.
        let fixes = vec![
            // Cluster A near (55.67, 12.55)
            fix("2026-05-09T12:00:00+00:00", 55.6684, 12.5541, 30.0),
            fix("2026-05-09T12:05:00+00:00", 55.6685, 12.5542, 30.0),
            fix("2026-05-09T12:10:00+00:00", 55.6683, 12.5540, 30.0),
            // Cluster B near (55.71, 12.55), ~5km north
            fix("2026-05-09T13:00:00+00:00", 55.7100, 12.5541, 30.0),
            fix("2026-05-09T13:05:00+00:00", 55.7101, 12.5540, 30.0),
            fix("2026-05-09T13:10:00+00:00", 55.7099, 12.5542, 30.0),
        ];
        let segs = segment_fixes(&fixes, 100.0, 60);
        assert_eq!(segs.len(), 2, "expected 2 segments, got {segs:?}");
        assert!((segs[0].centroid_lat - 55.6684).abs() < 1e-3);
        assert!((segs[1].centroid_lat - 55.7100).abs() < 1e-3);
    }

    #[test]
    fn segment_midnight_crossover_handled() {
        // Stay across UTC midnight: one segment with duration ≈ 30 min.
        let fixes = vec![
            fix("2026-05-09T23:45:00+00:00", 55.0, 12.0, 30.0),
            fix("2026-05-09T23:55:00+00:00", 55.0, 12.0, 30.0),
            fix("2026-05-10T00:05:00+00:00", 55.0, 12.0, 30.0),
            fix("2026-05-10T00:15:00+00:00", 55.0, 12.0, 30.0),
        ];
        let segs = segment_fixes(&fixes, 100.0, 60);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].duration_secs, 1800);
    }

    /// task-0087: one long stay used to run the median for membership
    /// prefixes 1..999 and then all 1,000 at flush: 500,500 coordinate
    /// inputs in total.
    /// The injectable seam proves the new structural cost directly: one
    /// invocation and exactly N coordinates for one retained segment.
    #[test]
    fn segment_long_stay_invokes_median_once_with_linear_input() {
        let fixes: Vec<_> = (0..1_000)
            .map(|_| fix("2026-05-09T12:00:00+00:00", 55.6684, 12.5541, 30.0))
            .collect();
        let mut calls = 0;
        let mut coordinate_inputs = 0;
        let mut never_cancelled = || false;
        let outcome = {
            let mut counted_median =
                |coords: &[(f64, f64)], _is_cancelled: &mut dyn FnMut() -> bool| {
                    calls += 1;
                    coordinate_inputs += coords.len();
                    match crate::trilaterate::geometric_median(coords) {
                        Some(point) => MedianComputation::Point(point),
                        None => MedianComputation::Degenerate,
                    }
                };
            segment_fixes_impl(&fixes, 100.0, 0, &mut never_cancelled, &mut counted_median)
        };

        let SegmentFixesOutcome::Complete(segments) = outcome else {
            panic!("non-cancelled segmentation unexpectedly cancelled");
        };
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].n_fixes, 1_000);
        assert_eq!(calls, 1, "one retained segment needs one median call");
        assert_eq!(
            coordinate_inputs, 1_000,
            "median input must be N, not the old 1+...+N amplification"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// task-0087 AC #3: for every generated partition, retained segments
        /// partition the input. Because the median runs only at flush, its
        /// total coordinate input is bounded by N and call count by N. This
        /// is a deterministic asymptotic invariant, not a machine-speed
        /// benchmark threshold.
        #[test]
        fn segment_median_work_is_structurally_linear(
            coords in prop::collection::vec((-89.0f64..89.0, -180.0f64..180.0), 0..96)
        ) {
            let fixes: Vec<_> = coords
                .iter()
                .map(|&(lat, lon)| fix("2026-05-09T12:00:00+00:00", lat, lon, 30.0))
                .collect();
            let mut calls = 0usize;
            let mut coordinate_inputs = 0usize;
            let mut never_cancelled = || false;
            let outcome = {
                let mut counted_median = |
                    segment_coords: &[(f64, f64)],
                    _is_cancelled: &mut dyn FnMut() -> bool,
                | {
                    calls += 1;
                    coordinate_inputs += segment_coords.len();
                    match crate::trilaterate::geometric_median(segment_coords) {
                        Some(point) => MedianComputation::Point(point),
                        None => MedianComputation::Degenerate,
                    }
                };
                segment_fixes_impl(
                    &fixes,
                    100_000.0,
                    0,
                    &mut never_cancelled,
                    &mut counted_median,
                )
            };

            prop_assert!(matches!(outcome, SegmentFixesOutcome::Complete(_)));
            prop_assert!(calls <= fixes.len());
            prop_assert!(coordinate_inputs <= fixes.len());
        }
    }

    /// Membership may use the cheap spherical mean, but the value exposed to
    /// clients must remain the outlier-robust geometric median.
    #[test]
    fn segment_reported_centroid_remains_outlier_robust_geometric_median() {
        let mut fixes = vec![
            fix("2026-05-09T12:00:00+00:00", 55.700, 12.580, 30.0),
            fix("2026-05-09T12:01:00+00:00", 55.701, 12.581, 30.0),
            fix("2026-05-09T12:02:00+00:00", 55.699, 12.579, 30.0),
            fix("2026-05-09T12:03:00+00:00", 55.700, 12.582, 30.0),
            fix("2026-05-09T12:04:00+00:00", 55.702, 12.580, 30.0),
        ];
        fixes.extend([
            fix("2026-05-09T12:05:00+00:00", -33.87, 151.21, 30.0),
            fix("2026-05-09T12:06:00+00:00", 35.68, 139.69, 30.0),
            fix("2026-05-09T12:07:00+00:00", -12.89, -38.29, 30.0),
            fix("2026-05-09T12:08:00+00:00", 40.71, -74.00, 30.0),
        ]);

        let segments = segment_fixes(&fixes, f64::INFINITY, 0);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].n_fixes, fixes.len());
        assert!(
            (segments[0].centroid_lat - 55.7).abs() < 1.0,
            "minority outliers displaced latitude: {}",
            segments[0].centroid_lat
        );
        assert!(
            (segments[0].centroid_lon - 12.58).abs() < 1.0,
            "minority outliers displaced longitude: {}",
            segments[0].centroid_lon
        );
    }

    #[test]
    fn segment_antimeridian_cluster_stays_together() {
        let fixes = vec![
            fix("2026-05-09T12:00:00+00:00", 10.0, 179.999, 20.0),
            fix("2026-05-09T12:01:00+00:00", 10.0, -179.999, 20.0),
            fix("2026-05-09T12:02:00+00:00", 10.001, 179.998, 20.0),
            fix("2026-05-09T12:03:00+00:00", 9.999, -179.998, 20.0),
        ];
        let segments = segment_fixes(&fixes, 1_000.0, 0);
        assert_eq!(segments.len(), 1, "antimeridian is not a segment break");
        assert_eq!(segments[0].n_fixes, fixes.len());
        assert!(
            segments[0].centroid_lon.abs() > 179.0,
            "centroid must remain near the antimeridian, got {}",
            segments[0].centroid_lon
        );
    }

    #[test]
    fn segment_polar_cluster_stays_together() {
        let fixes = vec![
            fix("2026-05-09T12:00:00+00:00", 89.999, 0.0, 20.0),
            fix("2026-05-09T12:01:00+00:00", 89.999, 120.0, 20.0),
            fix("2026-05-09T12:02:00+00:00", 89.999, -120.0, 20.0),
            fix("2026-05-09T12:03:00+00:00", 89.9995, 180.0, 20.0),
        ];
        let segments = segment_fixes(&fixes, 500.0, 0);
        assert_eq!(segments.len(), 1, "longitude collapses at the pole");
        assert_eq!(segments[0].n_fixes, fixes.len());
        assert!(
            segments[0].centroid_lat > 89.99,
            "polar centroid moved too far south: {}",
            segments[0].centroid_lat
        );
    }

    #[test]
    fn dropped_segment_never_marshals_or_invokes_median() {
        let fixes = vec![
            fix("2026-05-09T12:00:00+00:00", 55.0, 12.0, 20.0),
            fix("2026-05-09T12:00:30+00:00", 55.0, 12.0, 20.0),
        ];
        let mut never_cancelled = || false;
        let mut median = |_coords: &[(f64, f64)],
                          _is_cancelled: &mut dyn FnMut() -> bool|
         -> MedianComputation {
            panic!("a duration-dropped segment must not invoke the median")
        };
        assert_eq!(
            segment_fixes_impl(&fixes, 100.0, 60, &mut never_cancelled, &mut median),
            SegmentFixesOutcome::Complete(Vec::new())
        );
    }

    #[test]
    fn degenerate_running_mean_uses_finite_first_fix() {
        let first = fix("2026-05-09T12:00:00+00:00", 0.0, 0.0, 20.0);
        let antipode = fix("2026-05-09T12:01:00+00:00", 0.0, 180.0, 20.0);
        let mut mean = RunningSphericalMean::new(&first);
        mean.add(&antipode);
        let center = mean.center().expect("finite first-fix fallback");
        assert!(center.0.abs() < 1e-12);
        assert!(center.1.abs() < 1e-12);
    }

    #[test]
    fn non_finite_coordinates_never_produce_nan_centroids() {
        let fixes = vec![
            fix("2026-05-09T12:00:00+00:00", 55.0, 12.0, 20.0),
            fix("2026-05-09T12:01:00+00:00", f64::NAN, 12.0, 20.0),
            fix("2026-05-09T12:02:00+00:00", 55.0, 12.0, 20.0),
        ];
        let segments = segment_fixes(&fixes, f64::INFINITY, 0);
        assert!(!segments.is_empty());
        assert!(segments
            .iter()
            .all(|segment| segment.centroid_lat.is_finite() && segment.centroid_lon.is_finite()));
    }

    #[test]
    fn cancellation_during_linear_pass_is_explicit_and_prompt() {
        let fixes: Vec<_> = (0..1_000)
            .map(|_| fix("2026-05-09T12:00:00+00:00", 55.0, 12.0, 20.0))
            .collect();
        let mut checks = 0;
        let outcome = segment_fixes_cancellable(&fixes, 100.0, 0, || {
            checks += 1;
            checks >= 5
        });
        assert_eq!(outcome, SegmentFixesOutcome::Cancelled);
        assert_eq!(checks, 5, "cancellation must stop the linear pass promptly");
    }

    #[test]
    fn non_cancellable_wrapper_preserves_cancellable_core_output() {
        let fixes = vec![
            fix("2026-05-09T12:00:00+00:00", 55.0, 12.0, 20.0),
            fix("2026-05-09T12:05:00+00:00", 55.0001, 12.0001, 30.0),
            fix("2026-05-09T12:10:00+00:00", 55.0002, 12.0002, 40.0),
        ];
        let wrapper = segment_fixes(&fixes, 100.0, 60);
        assert_eq!(
            segment_fixes_cancellable(&fixes, 100.0, 60, || false),
            SegmentFixesOutcome::Complete(wrapper)
        );
    }

    // task-0141: observability tests for compute_geometric_median. A capped
    // (non-converged) iterate must emit a DEBUG event; an Antipodal/Degenerate
    // cluster must emit a WARN before falling back. Uses the shared
    // `test_support::capture` sink (task-0255).
    mod median_observability {
        use super::super::{compute_geometric_median, MedianComputation};
        use tracing::Level;

        /// Thin wrapper over the shared `test_support::capture` (task-0255):
        /// this module asserts on `(level, message)` only, so drop the unified
        /// sink's `target` column. Assertions are unchanged from when a local
        /// 2-field `CapturedEvents` sink lived here.
        fn capture<T>(operation: impl FnOnce() -> T) -> (T, Vec<(Level, String)>) {
            let (result, events) = crate::test_support::capture(operation);
            (
                result,
                events
                    .into_iter()
                    .map(|(level, _target, message)| (level, message))
                    .collect(),
            )
        }

        /// A cluster whose geometric median coincides with one of its input
        /// points converges very slowly under Weiszfeld (singular weight near
        /// the data point), so it exhausts WEISZFELD_MAX_ITERS -> Capped. Found
        /// empirically against the real `geometric_median_detailed`; the last
        /// iterate lands on ~(2.0, 15.0), i.e. the (2.0, 15.0) input point.
        const CAPPED_CLUSTER: &[(f64, f64)] = &[(0.0, 0.0), (5.0, 20.0), (-5.0, 10.0), (2.0, 15.0)];

        #[test]
        fn capped_median_emits_debug_and_still_returns_point() {
            let (result, events) =
                capture(|| compute_geometric_median(CAPPED_CLUSTER, &mut || false));

            // Behaviour is unchanged: the capped iterate is still used.
            assert!(
                matches!(result, MedianComputation::Point(_)),
                "capped iterate must still be returned as a Point, got {result:?}"
            );

            let debug_msgs: Vec<&String> = events
                .iter()
                .filter(|(level, _)| *level == Level::DEBUG)
                .map(|(_, msg)| msg)
                .collect();
            assert!(
                debug_msgs.iter().any(|m| m.contains("iteration cap")),
                "expected a DEBUG event about the Weiszfeld iteration cap; got {events:?}"
            );
        }

        /// Two diametrically opposed points cancel the spherical seed to
        /// magnitude ~0 -> Degenerate, which must warn before falling back.
        #[test]
        fn degenerate_median_emits_warn_and_falls_back() {
            let antipodal: &[(f64, f64)] = &[(0.0, 0.0), (0.0, 180.0)];
            let (result, events) = capture(|| compute_geometric_median(antipodal, &mut || false));

            assert!(
                matches!(result, MedianComputation::Degenerate),
                "antipodal pair must fall back to Degenerate, got {result:?}"
            );

            let warn_msgs: Vec<&String> = events
                .iter()
                .filter(|(level, _)| *level == Level::WARN)
                .map(|(_, msg)| msg)
                .collect();
            assert!(
                warn_msgs.iter().any(|m| m.contains("degenerate")),
                "expected a WARN event about the degenerate median; got {events:?}"
            );
        }
    }
}
