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

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::db::FixRow;

/// A grouped stay-point: a run of fixes within `dist_threshold_m` of each
/// other for at least `min_duration_secs`.
#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    /// First fix in the segment (RFC3339).
    pub start_rfc3339: String,
    /// Last fix in the segment (RFC3339).
    pub end_rfc3339: String,
    /// Duration in seconds.
    pub duration_secs: i64,
    /// Centroid latitude (mean of fix latitudes).
    pub centroid_lat: f64,
    /// Centroid longitude (mean of fix longitudes).
    pub centroid_lon: f64,
    /// Mean accuracy_m of fixes in the segment.
    pub mean_accuracy_m: f64,
    /// Number of fixes aggregated.
    pub n_fixes: usize,
}

/// Group consecutive fixes (already in ascending time order) into stay-point
/// segments. A new segment is started when the next fix is more than
/// `dist_threshold_m` from the running cluster centroid. Segments shorter
/// than `min_duration_secs` are discarded.
///
/// The fixes array MUST be sorted ascending by timestamp; the caller (DB
/// layer) does that.
pub fn segment_fixes(
    fixes: &[FixRow],
    dist_threshold_m: f64,
    min_duration_secs: i64,
) -> Vec<Segment> {
    if fixes.is_empty() {
        return Vec::new();
    }

    let mut segments = Vec::new();
    let mut cur: Vec<&FixRow> = vec![&fixes[0]];

    let flush = |cur: &[&FixRow], segments: &mut Vec<Segment>| {
        if cur.is_empty() {
            return;
        }
        let n = cur.len();
        let lat_sum: f64 = cur.iter().map(|f| f.lat).sum();
        let lon_sum: f64 = cur.iter().map(|f| f.lon).sum();
        let acc_sum: f64 = cur.iter().map(|f| f.accuracy_m).sum();
        let start = cur.first().unwrap();
        let end = cur.last().unwrap();
        // Duration: end - start (ts strings are RFC3339, parseable).
        let duration_secs = match (
            DateTime::parse_from_rfc3339(&start.at_rfc3339),
            DateTime::parse_from_rfc3339(&end.at_rfc3339),
        ) {
            (Ok(s), Ok(e)) => (e - s).num_seconds(),
            _ => 0,
        };
        if duration_secs < min_duration_secs {
            // Drop short segments — not a real stay-point.
            return;
        }
        segments.push(Segment {
            start_rfc3339: start.at_rfc3339.clone(),
            end_rfc3339: end.at_rfc3339.clone(),
            duration_secs,
            centroid_lat: lat_sum / n as f64,
            centroid_lon: lon_sum / n as f64,
            mean_accuracy_m: acc_sum / n as f64,
            n_fixes: n,
        });
    };

    for fix in fixes.iter().skip(1) {
        // Distance from running centroid to this fix.
        let n = cur.len() as f64;
        let cx: f64 = cur.iter().map(|f| f.lat).sum::<f64>() / n;
        let cy: f64 = cur.iter().map(|f| f.lon).sum::<f64>() / n;
        let d = haversine_m(cx, cy, fix.lat, fix.lon);
        if d <= dist_threshold_m {
            cur.push(fix);
        } else {
            flush(&cur, &mut segments);
            cur = vec![fix];
        }
    }
    flush(&cur, &mut segments);

    segments
}

// task-0081: haversine_m moved to crate::geo. Use it via the path below.
use crate::geo::haversine_m;

/// Parse a relative range string like "7d", "24h", "30m" into (start, end)
/// where end is now and start is now-duration. Also accepts "1w".
pub fn parse_range(spec: &str) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(anyhow!("empty range spec"));
    }
    // Split off the LAST CHARACTER (not last byte) so non-ASCII unit chars
    // don't panic on UTF-8 boundaries. task-0048.
    let last_char = spec
        .chars()
        .next_back()
        .ok_or_else(|| anyhow!("empty range spec"))?;
    let unit_len = last_char.len_utf8();
    let (num_str, unit) = spec.split_at(spec.len() - unit_len);
    let n: i64 = num_str
        .parse()
        .map_err(|_| anyhow!("invalid range '{spec}': expected '<number><unit>' (e.g. '7d')"))?;
    if n <= 0 {
        return Err(anyhow!("range '{spec}' must be > 0"));
    }
    let dur = match unit {
        "s" => Duration::seconds(n),
        "m" => Duration::minutes(n),
        "h" => Duration::hours(n),
        "d" => Duration::days(n),
        "w" => Duration::weeks(n),
        other => {
            return Err(anyhow!(
                "unknown range unit '{other}' in '{spec}': use s/m/h/d/w"
            ))
        }
    };
    let now = Utc::now();
    Ok((now - dur, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(ts: &str, lat: f64, lon: f64, acc: f64) -> FixRow {
        FixRow {
            id: 0,
            at_rfc3339: ts.to_string(),
            lat,
            lon,
            accuracy_m: acc,
            n_sources: 3,
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

    #[test]
    fn parse_range_understands_common_units() {
        let now_ts = Utc::now().timestamp();
        for spec in ["7d", "24h", "30m", "1w", "300s"] {
            let (start, end) = parse_range(spec).unwrap();
            assert!((end.timestamp() - now_ts).abs() < 5, "end ≈ now");
            assert!(start < end, "start {start:?} must be before end {end:?}");
        }
    }

    #[test]
    fn parse_range_rejects_garbage() {
        assert!(parse_range("").is_err());
        assert!(parse_range("7").is_err()); // no unit
        assert!(parse_range("xd").is_err());
        assert!(parse_range("7y").is_err()); // unknown unit
        assert!(parse_range("0d").is_err()); // must be > 0
        assert!(parse_range("-3d").is_err());
    }

    /// task-0048: parse_range used split_at(byte_len - 1) which panics on
    /// non-char-boundary inputs. Now uses last char's len_utf8.
    #[test]
    fn parse_range_no_panic_on_non_ascii() {
        // Multi-byte unit char: '日' is 3 bytes. Must NOT panic.
        assert!(parse_range("7日").is_err());
        // 4-byte char (emoji).
        assert!(parse_range("7🚀").is_err());
        // Pure non-ASCII string.
        assert!(parse_range("日").is_err());
        // Whitespace-only.
        assert!(parse_range("   ").is_err());
    }
}
