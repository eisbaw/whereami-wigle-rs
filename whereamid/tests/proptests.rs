//! Property-based tests using proptest (Hypothesis-style).

use proptest::prelude::*;

// Pull in the library code
use whereamid::debounce::{Debouncer, ScanEntry, ScanSample};
use whereamid::geo::haversine_m as haversine_m_test;
use whereamid::scanner::{normalize_bssid, parse_iw_output, split_nmcli_fields};
use whereamid::trilaterate::{filter_outliers, trilaterate, PositionedAp};
// task-0081: haversine consolidated into whereamid::geo. We re-import as
// haversine_m_test to avoid touching the existing test bodies.

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

    /// Trilateration result lies within the spherical convex hull of input APs:
    /// its haversine distance to every input AP is bounded by the diameter of
    /// the input cluster.
    ///
    /// This is the spherically-correct version of "within the bounding box".
    /// The lat/lon bounding-box property is wrong on a sphere — a great-circle
    /// arc between two points can extend to higher latitudes than either
    /// endpoint (think a flight London→Tokyo passing over Siberia).
    #[test]
    fn trilaterate_within_cluster_diameter(
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

        // Compute the diameter of the input cluster (max pairwise haversine).
        let mut diameter = 0.0_f64;
        for i in 0..filtered.len() {
            for j in (i + 1)..filtered.len() {
                let d = haversine_m_test(filtered[i].lat, filtered[i].lon, filtered[j].lat, filtered[j].lon);
                if d > diameter { diameter = d; }
            }
        }

        // The centroid is no farther from any AP than the cluster diameter
        // (with 1m slack for floating-point). This is a tight spherical-convex-hull bound.
        for ap in &filtered {
            let d = haversine_m_test(result.lat, result.lon, ap.lat, ap.lon);
            prop_assert!(d <= diameter + 1.0,
                "centroid->AP haversine {:.3}m exceeds cluster diameter {:.3}m", d, diameter);
        }
    }

    /// Stronger-signal AP pulls the centroid toward itself.
    ///
    /// Generate one strong AP and one weak AP at distinct positions and assert
    /// the centroid is closer to the strong one. This is the actual purpose
    /// of the dBm-weighted formula. Replaces the previous tautological
    /// `trilaterate_accuracy_positive` (clamped output is trivially > 0).
    #[test]
    fn trilaterate_stronger_signal_pulls_centroid(
        lat_a in 50.0f64..51.0,
        lon_a in 10.0f64..11.0,
        offset_lat in 0.005f64..0.05,
        offset_lon in 0.005f64..0.05,
        strong in -50i32..-30,
        weak in -85i32..-70,
    ) {
        let strong_ap = PositionedAp {
            lat: lat_a,
            lon: lon_a,
            signal_dbm: Some(strong),
        };
        let weak_ap = PositionedAp {
            lat: lat_a + offset_lat,
            lon: lon_a + offset_lon,
            signal_dbm: Some(weak),
        };
        let aps = vec![strong_ap.clone(), weak_ap.clone()];
        let result = trilaterate(&aps).unwrap();

        let d_strong = haversine_m_test(result.lat, result.lon, strong_ap.lat, strong_ap.lon);
        let d_weak = haversine_m_test(result.lat, result.lon, weak_ap.lat, weak_ap.lon);
        prop_assert!(
            d_strong < d_weak,
            "centroid should be closer to strong AP ({} dBm) than weak ({} dBm); \
             d_strong={:.1}m, d_weak={:.1}m",
            strong, weak, d_strong, d_weak
        );
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

    /// parse_iw_output: panic-freedom plus a shape invariant. Every emitted
    /// network must have a well-formed BSSID (six colon-separated hex bytes).
    /// Without the shape check this would only validate that safe-Rust code
    /// doesn't panic on parser input — almost no signal.
    #[test]
    fn parse_iw_output_emits_well_formed_bssids(input in ".*") {
        let networks = parse_iw_output(&input);
        for n in &networks {
            let parts: Vec<&str> = n.bssid.split(':').collect();
            prop_assert_eq!(
                parts.len(), 6,
                "iw parser emitted BSSID with {} colon-segments: {:?}",
                parts.len(), n.bssid
            );
            for p in parts {
                prop_assert!(
                    p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()),
                    "iw parser emitted non-hex BSSID octet {:?} in {}", p, n.bssid
                );
            }
        }
    }

    /// split_nmcli_fields panic-freedom plus a shape invariant. The output
    /// must never contain a literal "\:" escape sequence — those are the
    /// inputs being escaped.
    #[test]
    fn split_nmcli_no_unescaped_backslash_colon(input in ".*") {
        let fields = split_nmcli_fields(&input);
        for f in &fields {
            prop_assert!(
                !f.contains("\\:"),
                "split_nmcli_fields left an unprocessed \\: escape in field {:?}", f
            );
        }
    }

    /// split_nmcli_fields correctly splits on unescaped colons even when
    /// fields contain backslashes and escaped colons. The previous version
    /// of this property tested only `[a-zA-Z0-9]{0,10}`, which never
    /// exercised the escape logic — a vacuous regression.
    #[test]
    fn split_nmcli_round_trips_with_escapes(
        // Each "field" is built from raw chars that can include literal ':',
        // which we'll then escape as '\:' to compose a wire-format line.
        // We exclude '\\' itself so we don't have to model lookahead in
        // round-trip; the escape semantics under '\\' are tested in the unit
        // test for split_nmcli_fields directly.
        fields in prop::collection::vec("[a-zA-Z0-9: ]{0,10}", 1..6)
    ) {
        // Compose: every literal ':' in a field becomes '\:' on the wire.
        let line = fields
            .iter()
            .map(|f| f.replace(':', "\\:"))
            .collect::<Vec<_>>()
            .join(":");
        let split = split_nmcli_fields(&line);
        prop_assert_eq!(split.len(), fields.len(),
            "expected {} fields, got {}; line was {:?}", fields.len(), split.len(), line);
        for (orig, parsed) in fields.iter().zip(split.iter()) {
            prop_assert_eq!(orig, parsed,
                "round-trip mismatch: original {:?} != parsed {:?}", orig, parsed);
        }
    }
}

// --- Debounce properties ---

proptest! {
    /// Two non-trivial debounce invariants:
    ///   1. After at least `threshold` scans containing every AP, every AP
    ///      must be stable (the basic threshold contract).
    ///   2. After fewer than `threshold` scans, NO AP can be stable.
    /// The previous `debounce_stable_bounded` property (`stable.len() <=
    /// num_aps`) was definitionally true for any HashMap-backed debouncer
    /// and provided no real signal.
    #[test]
    fn debounce_threshold_contract(
        window in 3usize..15,
        threshold in 2usize..5,
        num_aps in 1usize..6,
    ) {
        let threshold = threshold.min(window);
        let mut d = Debouncer::new(window, threshold);

        // Push exactly threshold-1 scans containing every AP — none stable.
        for _ in 0..(threshold - 1) {
            let mut sample: ScanSample = std::collections::HashMap::new();
            for i in 0..num_aps {
                sample.insert(format!("AP-{i}"), ScanEntry { signal_dbm: -60, ssid: None, channel: None });
            }
            d.push_scan(sample);
        }
        prop_assert!(
            d.stable_bssids().is_empty(),
            "no AP should be stable after {} scans (threshold = {})",
            threshold - 1, threshold
        );

        // One more scan crosses the threshold — every AP must be stable now.
        let mut sample: ScanSample = std::collections::HashMap::new();
        for i in 0..num_aps {
            sample.insert(format!("AP-{i}"), ScanEntry { signal_dbm: -60, ssid: None, channel: None });
        }
        d.push_scan(sample);

        let stable = d.stable_bssids();
        prop_assert_eq!(
            stable.len(), num_aps,
            "after {} scans (threshold={}), every AP must be stable",
            threshold, threshold
        );
        for i in 0..num_aps {
            let key = format!("AP-{i}");
            prop_assert!(stable.contains(&key), "AP-{} should be stable", i);
        }
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
