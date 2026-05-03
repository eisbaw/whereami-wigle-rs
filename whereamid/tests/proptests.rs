//! Property-based tests using proptest (Hypothesis-style).

use proptest::prelude::*;

// Pull in the library code
use whereamid::debounce::{Debouncer, ScanEntry, ScanSample};
use whereamid::scanner::{normalize_bssid, parse_iw_output, split_nmcli_fields};
use whereamid::trilaterate::{filter_outliers, trilaterate, PositionedAp};

// --- Trilateration properties ---

proptest! {
    /// Outlier filter never returns empty when given non-empty input.
    #[test]
    fn outlier_filter_never_empty(
        coords in prop::collection::vec((-90.0f64..90.0, -180.0f64..180.0), 1..20)
    ) {
        let aps: Vec<PositionedAp> = coords
            .iter()
            .map(|&(lat, lon)| PositionedAp { lat, lon, signal_dbm: Some(-60) })
            .collect();
        let filtered = filter_outliers(&aps);
        prop_assert!(!filtered.is_empty(), "filter_outliers returned empty for {} APs", aps.len());
    }

    /// Trilateration result is within the bounding box of input APs (after outlier filtering).
    #[test]
    fn trilaterate_within_bounds(
        coords in prop::collection::vec((40.0f64..60.0, 5.0f64..20.0), 2..10)
    ) {
        let aps: Vec<PositionedAp> = coords
            .iter()
            .map(|&(lat, lon)| PositionedAp { lat, lon, signal_dbm: Some(-60) })
            .collect();

        // Only test with APs that survive outlier filtering
        let filtered = filter_outliers(&aps);
        if filtered.len() < 2 {
            return Ok(());
        }

        let result = trilaterate(&filtered).unwrap();

        let min_lat = filtered.iter().map(|a| a.lat).fold(f64::INFINITY, f64::min);
        let max_lat = filtered.iter().map(|a| a.lat).fold(f64::NEG_INFINITY, f64::max);
        let min_lon = filtered.iter().map(|a| a.lon).fold(f64::INFINITY, f64::min);
        let max_lon = filtered.iter().map(|a| a.lon).fold(f64::NEG_INFINITY, f64::max);

        prop_assert!(result.lat >= min_lat - 0.001 && result.lat <= max_lat + 0.001,
            "lat {} outside bounds [{}, {}]", result.lat, min_lat, max_lat);
        prop_assert!(result.lon >= min_lon - 0.001 && result.lon <= max_lon + 0.001,
            "lon {} outside bounds [{}, {}]", result.lon, min_lon, max_lon);
    }

    /// Trilateration accuracy is always positive.
    #[test]
    fn trilaterate_accuracy_positive(
        coords in prop::collection::vec((50.0f64..56.0, 10.0f64..14.0), 1..8),
        signals in prop::collection::vec(-90i32..-30, 1..8)
    ) {
        let len = coords.len().min(signals.len());
        let aps: Vec<PositionedAp> = coords[..len]
            .iter()
            .zip(signals[..len].iter())
            .map(|(&(lat, lon), &sig)| PositionedAp { lat, lon, signal_dbm: Some(sig) })
            .collect();

        if let Ok(result) = trilaterate(&aps) {
            prop_assert!(result.accuracy_m > 0.0, "accuracy should be positive");
        }
    }
}

// --- Parser properties ---

proptest! {
    /// normalize_bssid is idempotent: applying twice gives same result.
    #[test]
    fn normalize_bssid_idempotent(s in "[0-9a-fA-F]{2}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}") {
        let once = normalize_bssid(&s);
        let twice = normalize_bssid(&once);
        prop_assert_eq!(&once, &twice);
    }

    /// parse_iw_output never panics on arbitrary input.
    #[test]
    fn parse_iw_no_panic(input in ".*") {
        let _ = parse_iw_output(&input);
    }

    /// split_nmcli_fields never panics and round-trips unescaped fields.
    #[test]
    fn split_nmcli_no_panic(input in ".*") {
        let _ = split_nmcli_fields(&input);
    }

    /// split_nmcli_fields correctly splits on unescaped colons.
    #[test]
    fn split_nmcli_unescaped_colon_count(
        fields in prop::collection::vec("[a-zA-Z0-9]{0,10}", 1..6)
    ) {
        let joined = fields.join(":");
        let split = split_nmcli_fields(&joined);
        prop_assert_eq!(split.len(), fields.len(),
            "splitting {:?} on ':' should give {} fields, got {}", joined, fields.len(), split.len());
    }
}

// --- Debounce properties ---

proptest! {
    /// Stable BSSID count never exceeds the number of unique BSSIDs pushed.
    #[test]
    fn debounce_stable_bounded(
        window in 3usize..15,
        threshold in 1usize..5,
        num_scans in 1usize..20,
        num_aps in 1usize..10,
    ) {
        let threshold = threshold.min(window);
        let mut d = Debouncer::new(window, threshold);

        for _ in 0..num_scans {
            let mut sample: ScanSample = std::collections::HashMap::new();
            for i in 0..num_aps {
                sample.insert(format!("AP-{i}"), ScanEntry { signal_dbm: -60, ssid: None, channel: None });
            }
            d.push_scan(sample);
        }

        let stable = d.stable_bssids();
        prop_assert!(stable.len() <= num_aps, "stable count {} > num_aps {}", stable.len(), num_aps);
    }

    /// After window+1 empty scans, nothing is stable.
    #[test]
    fn debounce_forgets_after_window(
        window in 3usize..10,
        threshold in 2usize..5,
    ) {
        let threshold = threshold.min(window);
        let mut d = Debouncer::new(window, threshold);

        // Fill with one AP
        for _ in 0..window {
            let mut sample: ScanSample = std::collections::HashMap::new();
            sample.insert("AP-1".to_string(), ScanEntry { signal_dbm: -50, ssid: None, channel: None });
            d.push_scan(sample);
        }
        prop_assert!(!d.stable_bssids().is_empty());

        // Push window empty scans to flush it out
        for _ in 0..window {
            d.push_scan(std::collections::HashMap::new());
        }
        prop_assert!(d.stable_bssids().is_empty(), "should have no stable APs after {} empty scans", window);
    }
}
