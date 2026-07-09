//! Property-based tests using proptest (Hypothesis-style).

use proptest::prelude::*;

// Pull in the library code
use whereamid::debounce::{Debouncer, ScanEntry, ScanSample};
use whereamid::geo::haversine_m as haversine_m_test;
use whereamid::rssi::RssiSource;
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
            .map(|&(lat, lon)| PositionedAp { lat, lon, signal_dbm: Some(-60), rssi_source: RssiSource::IwDbm, is_mobile: false })
            .collect();
        let filtered = filter_outliers(&aps);
        prop_assert!(!filtered.is_empty(), "filter_outliers returned empty for {} APs", aps.len());
    }

    /// task-0185: haversine_m must stay finite for near-antipodal inputs.
    /// The antipode of (lat1, lon1) is (-lat1, lon1±180); jittering both
    /// points by up to ~1e-6° around that makes the haversine `a` term
    /// straddle 1.0 — where the un-clamped formula returned NaN via
    /// asin(x>1). Every result must be finite and within [0, πR].
    #[test]
    fn haversine_finite_near_antipodal(
        lat1 in -90.0f64..90.0,
        lon1 in -180.0f64..180.0,
        dlat in -1e-6f64..1e-6,
        dlon in -1e-6f64..1e-6,
    ) {
        const R: f64 = 6_371_000.0;
        let max = std::f64::consts::PI * R;
        let lat2 = -lat1 + dlat;
        let mut lon2 = lon1 + 180.0 + dlon;
        if lon2 > 180.0 {
            lon2 -= 360.0;
        }
        let d = haversine_m_test(lat1, lon1, lat2, lon2);
        prop_assert!(
            d.is_finite(),
            "haversine non-finite at near-antipode: ({lat1},{lon1})->({lat2},{lon2}) = {d}"
        );
        prop_assert!(
            (0.0..=max + 1.0).contains(&d),
            "haversine {d} outside [0, piR={max}]"
        );
    }

    /// The PUBLIC trilaterate() rejects a raw outlier and keeps its result
    /// inside the tight cluster's spherical convex hull.
    ///
    /// task-0153: this test feeds RAW, outlier-laden input straight to
    /// `trilaterate` — it does NOT pre-apply `filter_outliers`. Because
    /// `trilaterate` runs `filter_outliers` internally (mod.rs:207), pre-
    /// filtering in the test would only exercise the inner centroid math and
    /// a refactor disabling the internal filter would still pass. Feeding the
    /// outlier straight in makes the internal filter load-bearing: if it is
    /// removed, the far outlier pulls the centroid far outside the cluster
    /// diameter and this test fails (verified by mutation, see task notes).
    ///
    /// Generation guarantees (so the invariant holds for EVERY input and the
    /// test never flakes):
    ///   * A TIGHT cluster of ≥3 APs: a center in a safe mid-latitude/-longitude
    ///     range (no pole/antimeridian degeneracy) plus ±0.004° (~≤450 m)
    ///     jitter per AP. Max intra-cluster pairwise distance stays well under
    ///     `NEIGHBOR_RADIUS_M` (2000 m) at every latitude in range, so stage-1
    ///     `drop_isolated` KEEPS the whole cluster and the "every AP isolated"
    ///     bypass never fires.
    ///   * ONE outlier ~5° (~555 km) away in latitude — far beyond 2000 m from
    ///     every cluster AP, so it is isolated and dropped by the internal
    ///     `filter_outliers`.
    ///
    /// The bound (dist(centroid, any cluster AP) ≤ cluster diameter) is the
    /// spherically-correct version of "within the bounding box": on a sphere a
    /// great-circle arc between two points can reach higher latitudes than
    /// either endpoint, so a lat/lon box bound is wrong. The centroid lies in
    /// the spherical convex hull of the surviving APs (a subset of the
    /// cluster), hence within the full cluster's diameter.
    #[test]
    fn trilaterate_within_cluster_diameter(
        center_lat in -80.0f64..80.0,
        center_lon in -170.0f64..170.0,
        // ≥3 cluster APs, each a small jitter offset from the center.
        jitters in prop::collection::vec((-0.004f64..0.004, -0.004f64..0.004), 3..8),
    ) {
        // Tight cluster: center + per-AP jitter. All within ~1.3 km of each
        // other, so mutually within NEIGHBOR_RADIUS_M (2000 m).
        let cluster_aps: Vec<PositionedAp> = jitters
            .iter()
            .map(|&(dlat, dlon)| PositionedAp {
                lat: center_lat + dlat,
                lon: center_lon + dlon,
                signal_dbm: Some(-60),
                rssi_source: RssiSource::IwDbm,
                is_mobile: false,
            })
            .collect();

        // One far outlier ~5° north of the center (~555 km): isolated, so the
        // internal filter_outliers drops it. center_lat ≤ 80 → outlier ≤ 85,
        // still finite and far from the pole.
        let outlier = PositionedAp {
            lat: center_lat + 5.0,
            lon: center_lon,
            signal_dbm: Some(-60),
            rssi_source: RssiSource::IwDbm,
            is_mobile: false,
        };

        // RAW input handed to the PUBLIC API — no pre-filtering.
        let mut raw_aps = cluster_aps.clone();
        raw_aps.push(outlier);

        let result = trilaterate(&raw_aps).unwrap();

        // Diameter of the CLUSTER only (max pairwise haversine), excluding the
        // outlier. If the internal filter fails to drop the outlier, the
        // centroid is pulled ~toward it and blows past this bound.
        let mut diameter = 0.0_f64;
        for i in 0..cluster_aps.len() {
            for j in (i + 1)..cluster_aps.len() {
                let d = haversine_m_test(cluster_aps[i].lat, cluster_aps[i].lon, cluster_aps[j].lat, cluster_aps[j].lon);
                if d > diameter { diameter = d; }
            }
        }

        // The centroid is no farther from any cluster AP than the cluster
        // diameter (with a few metres of slack for floating-point noise only —
        // NOT enough to absorb an undropped 555 km outlier, whose pull is tens
        // to hundreds of km). This is the tight spherical-convex-hull bound.
        for ap in &cluster_aps {
            let d = haversine_m_test(result.lat, result.lon, ap.lat, ap.lon);
            prop_assert!(d <= diameter + 2.0,
                "centroid->cluster-AP haversine {:.3}m exceeds cluster diameter {:.3}m (outlier not dropped by internal filter?)", d, diameter);
        }
    }

    /// task-0186 AC4: with a MIX of measured and signal-less APs, the centroid
    /// must stay within the cluster diameter (a None AP must not run away with
    /// it) and every output must be finite. This is the spherical-convex-hull
    /// bound from `trilaterate_within_cluster_diameter` extended to the
    /// previously-untested None path.
    #[test]
    fn trilaterate_mixed_signal_within_cluster_diameter(
        coords in prop::collection::vec(
            (40.0f64..60.0, 5.0f64..20.0, prop::option::of(-100i32..-30)),
            2..10,
        )
    ) {
        let aps: Vec<PositionedAp> = coords
            .iter()
            .map(|&(lat, lon, sig)| PositionedAp { lat, lon, signal_dbm: sig, rssi_source: RssiSource::IwDbm, is_mobile: false })
            .collect();

        let filtered = filter_outliers(&aps);
        if filtered.len() < 2 {
            return Ok(());
        }
        let result = trilaterate(&filtered).unwrap();
        prop_assert!(
            result.lat.is_finite() && result.lon.is_finite() && result.accuracy_m.is_finite(),
            "non-finite result: ({}, {}, {})", result.lat, result.lon, result.accuracy_m
        );

        let mut diameter = 0.0_f64;
        for i in 0..filtered.len() {
            for j in (i + 1)..filtered.len() {
                let d = haversine_m_test(filtered[i].lat, filtered[i].lon, filtered[j].lat, filtered[j].lon);
                if d > diameter { diameter = d; }
            }
        }
        for ap in &filtered {
            let d = haversine_m_test(result.lat, result.lon, ap.lat, ap.lon);
            prop_assert!(d <= diameter + 1.0,
                "centroid->AP haversine {:.3}m exceeds cluster diameter {:.3}m", d, diameter);
        }
    }

    /// task-0227 AC#3: injecting a NaN / ±Inf coordinate AP into a finite
    /// cluster must never surface as a non-finite `Ok(Position)`. trilaterate
    /// drops non-finite inputs at its entry (defense in depth behind the
    /// server-side cache-read filter), so the poisoned AP — despite a strong
    /// signal meant to dominate the weighted centroid — contributes nothing.
    /// This is the property form of the `non_finite_ap_excluded_from_cluster`
    /// unit test: it must hold for ANY finite cluster and ANY non-finite
    /// injection site (lat or lon).
    #[test]
    fn trilaterate_excludes_non_finite_coord_ap(
        coords in prop::collection::vec((40.0f64..60.0, 5.0f64..20.0), 2..8),
        bad in prop_oneof![
            Just(f64::NAN),
            Just(f64::INFINITY),
            Just(f64::NEG_INFINITY),
        ],
        bad_in_lat in prop::bool::ANY,
    ) {
        let mut aps: Vec<PositionedAp> = coords
            .iter()
            .map(|&(lat, lon)| PositionedAp { lat, lon, signal_dbm: Some(-60), rssi_source: RssiSource::IwDbm, is_mobile: false })
            .collect();
        // One poisoned AP with a strong signal (-30 dBm) that WOULD dominate
        // the weighted centroid if it reached the math.
        let poisoned = if bad_in_lat {
            PositionedAp { lat: bad, lon: 12.0, signal_dbm: Some(-30), rssi_source: RssiSource::IwDbm, is_mobile: false }
        } else {
            PositionedAp { lat: 55.0, lon: bad, signal_dbm: Some(-30), rssi_source: RssiSource::IwDbm, is_mobile: false }
        };
        aps.push(poisoned);

        // The finite cluster (>= 2 APs) survives the entry filter, so this is
        // always Ok — never the empty-input Err.
        let result = trilaterate(&aps).unwrap();
        prop_assert!(
            result.lat.is_finite() && result.lon.is_finite() && result.accuracy_m.is_finite(),
            "non-finite result ({}, {}, {}) despite finite cluster + one {bad} AP",
            result.lat, result.lon, result.accuracy_m
        );
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
            rssi_source: RssiSource::IwDbm,
            is_mobile: false,
        };
        let weak_ap = PositionedAp {
            lat: lat_a + offset_lat,
            lon: lon_a + offset_lon,
            signal_dbm: Some(weak),
            rssi_source: RssiSource::IwDbm,
            is_mobile: false,
        };
        let aps = vec![strong_ap, weak_ap];
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

    /// split_nmcli_fields is TOTAL: it never panics on arbitrary input,
    /// including malformed escape sequences, and always yields at least one
    /// field. Panic-freedom on arbitrary bytes is otherwise only asserted by
    /// the `cfg(kani)` harness, which is not run in normal CI.
    ///
    /// (This replaces an earlier `!field.contains("\\:")` assertion, which
    /// was a false-positive proxy: correct decoding of the wire sequence
    /// `\\` followed by `\:` legitimately yields a field containing a literal
    /// backslash-colon, so that assertion flagged correct output as a failure
    /// and made `just e2e` flakily red. task-0226.)
    #[test]
    fn split_nmcli_fields_is_total(input in ".*") {
        let fields = split_nmcli_fields(&input);
        prop_assert!(!fields.is_empty(), "must always yield at least one field");
    }

    /// split_nmcli_fields is the exact inverse of nmcli terse escaping over
    /// the FULL escape set: a literal '\' is escaped as '\\' and a literal
    /// ':' as '\:'. The field alphabet now INCLUDES '\\' — the earlier
    /// version excluded it "to avoid modelling lookahead", which dodged the
    /// exact `\\` handling that task-0226 fixed.
    #[test]
    fn split_nmcli_round_trips_with_escapes(
        fields in prop::collection::vec("[a-zA-Z0-9:\\\\ ]{0,10}", 1..6)
    ) {
        // Compose the wire form the way nmcli does: escape '\' -> '\\' FIRST,
        // then ':' -> '\:' (order matters: escaping ':' first would introduce
        // backslashes that the '\' pass would then double-escape).
        let line = fields
            .iter()
            .map(|f| f.replace('\\', "\\\\").replace(':', "\\:"))
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
                sample.insert(format!("AP-{i}"), ScanEntry { signal_dbm: -60, rssi_source: RssiSource::IwDbm, ssid: None, channel: None });
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
            sample.insert(format!("AP-{i}"), ScanEntry { signal_dbm: -60, rssi_source: RssiSource::IwDbm, ssid: None, channel: None });
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
            sample.insert("AP-1".to_string(), ScanEntry { signal_dbm: -50, rssi_source: RssiSource::IwDbm, ssid: None, channel: None });
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
