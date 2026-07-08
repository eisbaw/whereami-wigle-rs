//! Wire-format guard for the shared protocol types (task-0114).
//!
//! These tests are the single, bidirectional proof that serialization (the
//! daemon side) and deserialization (the client side) agree, now that both
//! link one definition. They replace the previously duplicated per-crate
//! suites (the client's `stats_wire_tests`, `wire_enum_tests`, and the
//! task-0152 `daemon_wire_cross_tests` fixtures).
//!
//! Three layers:
//!   1. `roundtrip` — construct a representative value, serialize, deserialize,
//!      assert equality. Structurally guarantees serialize/deserialize agree.
//!   2. `wire_shape` — pin the exact bytes for skip/emit/null/rename behavior,
//!      proving the extraction did not change the wire.
//!   3. `daemon_fixtures` — byte-exact strings hand-derived from the daemon's
//!      emit path decode into the typed responses (carried from task-0152).
//!
//! FLOAT NOTE: this workspace builds serde_json WITHOUT `float_roundtrip`, so
//! its default parser is not correctly rounded and can land 1 ULP from a
//! literal parsed by rustc. Round-trip fixtures therefore use only
//! exactly-representable dyadic floats (e.g. 55.5, 0.75, 42.5) so whole-struct
//! `assert_eq!` is safe; the daemon fixtures (which carry real coordinates like
//! 55.6761) use a relative-tolerance compare instead.

use whereami_protocol::*;

/// Serialize then deserialize; the value must survive unchanged.
#[track_caller]
fn round_trip<T>(value: &T)
where
    T: serde::Serialize + for<'de> serde::Deserialize<'de> + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value).expect("serialize");
    let back: T = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(
        *value, back,
        "round-trip changed the value; json was {json}"
    );
}

mod roundtrip {
    use super::*;

    #[test]
    fn error_response() {
        round_trip(&ErrorResponse {
            ok: false,
            v: 1,
            error: "unknown command".to_string(),
        });
    }

    #[test]
    fn locate_response_full_and_minimal() {
        round_trip(&LocateResponse {
            ok: true,
            v: 1,
            lat: 55.5,
            lon: 12.25,
            accuracy_m: 42.5,
            sources: 7,
            cached: 5,
            visible: 9,
            stable: 6,
            address: Some("Copenhagen".to_string()),
            stale: true,
            fallback: true,
            age_s: Some(120),
            implied_speed_kmh: Some(13.5),
            last_fix_age_secs: Some(30),
            inlier_ratio: Some(0.75),
            confidence: Confidence::Medium,
            suspect_reason: Some("speed".to_string()),
            error: None,
        });
        // Minimal / error envelope: every optional absent.
        round_trip(&LocateResponse {
            ok: false,
            v: 1,
            lat: 0.0,
            lon: 0.0,
            accuracy_m: 0.0,
            sources: 0,
            cached: 0,
            visible: 0,
            stable: 0,
            address: None,
            stale: false,
            fallback: false,
            age_s: None,
            implied_speed_kmh: None,
            last_fix_age_secs: None,
            inlier_ratio: None,
            confidence: Confidence::High,
            suspect_reason: None,
            error: Some("trilateration failed".to_string()),
        });
    }

    #[test]
    fn resolve_response_and_entries() {
        round_trip(&ResolveResponse {
            ok: true,
            v: 1,
            results: vec![
                ResolveResultEntry {
                    bssid: "02:00:00:00:00:01".to_string(),
                    lat: Some(55.5),
                    lon: Some(12.25),
                    ssid: Some("cafe".to_string()),
                    provenance: Provenance::Cache,
                },
                ResolveResultEntry {
                    bssid: "02:00:00:00:00:02".to_string(),
                    lat: None,
                    lon: None,
                    ssid: None,
                    provenance: Provenance::NotFound,
                },
            ],
            error: None,
        });
    }

    #[test]
    fn scan_response() {
        round_trip(&ScanResponse {
            ok: true,
            v: 1,
            networks: vec![NetworkEntry {
                bssid: "02:00:00:00:00:0a".to_string(),
                ssid: Some("home".to_string()),
                signal_dbm: -52,
                channel: Some(6),
            }],
            scan_age_ms: Some(1500),
            scanned_at: Some("2026-01-02T03:04:05Z".to_string()),
            error: None,
        });
    }

    #[test]
    fn stats_response() {
        round_trip(&StatsResponse {
            ok: true,
            v: 1,
            cached_aps: 100,
            pending_aps: 5,
            not_found_aps: 3,
            db_size_bytes: 40960,
            api_calls_today: 12,
            api_counter_corruption_denials: 3,
            inflight: 2,
            db_write_failures: 1,
            db_read_failures: 4,
            autolocate_fixes_written: 7,
            orphan_tasks_aborted: 0,
            error: None,
        });
    }

    #[test]
    fn debug_response() {
        round_trip(&DebugResponse {
            ok: true,
            v: 1,
            daemon_rev: Some("abc1234".to_string()),
            scan_age_ms: Some(800),
            samples_in_buffer: 4,
            visible: 3,
            stable: 2,
            bssids: vec![
                DebugBssid {
                    bssid: "02:00:00:00:00:aa".to_string(),
                    signal_dbm: Some(-48),
                    seen: 5,
                    needed: 3,
                    is_stable: true,
                    db_status: DbStatus::Cached,
                },
                DebugBssid {
                    bssid: "02:00:00:00:00:bb".to_string(),
                    signal_dbm: None,
                    seen: 2,
                    needed: 3,
                    is_stable: false,
                    db_status: DbStatus::New,
                },
            ],
            error: None,
        });
    }

    #[test]
    fn version_response() {
        round_trip(&VersionResponse {
            ok: true,
            v: 1,
            version: Some("0.1.0".to_string()),
            git_rev: Some("deadbeef".to_string()),
            error: None,
        });
    }

    #[test]
    fn history_response_and_segment() {
        round_trip(&HistorySegment {
            start_rfc3339: "2026-01-01T08:00:00Z".to_string(),
            end_rfc3339: "2026-01-01T09:30:00Z".to_string(),
            duration_secs: 5400,
            centroid_lat: 55.5,
            centroid_lon: 12.25,
            mean_accuracy_m: 35.0,
            n_fixes: 42,
        });
        round_trip(&HistoryResponse {
            ok: true,
            v: 1,
            from: "2026-01-01T00:00:00Z".to_string(),
            to: "2026-01-02T00:00:00Z".to_string(),
            segments: vec![HistorySegment {
                start_rfc3339: "2026-01-01T08:00:00Z".to_string(),
                end_rfc3339: "2026-01-01T09:30:00Z".to_string(),
                duration_secs: 5400,
                centroid_lat: 55.5,
                centroid_lon: 12.25,
                mean_accuracy_m: 35.0,
                n_fixes: 42,
            }],
            error: None,
        });
    }

    #[test]
    fn history_fixes_response_range_and_fix() {
        round_trip(&HistoryRange {
            from_rfc3339: "a".to_string(),
            to_rfc3339: "b".to_string(),
        });
        // Legacy fix (all quality optionals absent) and a full fix.
        round_trip(&HistoryFix {
            at_rfc3339: "2026-05-09T12:00:00+00:00".to_string(),
            lat: 55.5,
            lon: 12.25,
            accuracy_m: 30.0,
            n_sources: 4,
            implied_speed_kmh: None,
            last_fix_age_secs: None,
            inlier_ratio: None,
            confidence: None,
            suspect_reason: None,
        });
        round_trip(&HistoryFixesResponse {
            ok: true,
            v: 1,
            range: HistoryRange {
                from_rfc3339: "a".to_string(),
                to_rfc3339: "b".to_string(),
            },
            fixes: vec![HistoryFix {
                at_rfc3339: "new".to_string(),
                lat: 5.0,
                lon: 6.0,
                accuracy_m: 7.0,
                n_sources: 8,
                implied_speed_kmh: Some(9.5),
                last_fix_age_secs: Some(10),
                inlier_ratio: Some(0.75),
                confidence: Some(Confidence::Suspect),
                suspect_reason: Some("exact".to_string()),
            }],
            elided: Some(2),
            error: None,
        });
    }

    #[test]
    fn wipe_response() {
        round_trip(&WipeResponse {
            ok: true,
            v: 1,
            aps_deleted: 5,
            pending_deleted: 4,
            not_found_deleted: 3,
            fixes_deleted: 2,
            last_fix_deleted: 1,
            error: None,
        });
    }
}

mod enums {
    use super::*;

    #[test]
    fn confidence_vocabulary_display_and_default() {
        for (variant, wire, display) in [
            (Confidence::High, "\"high\"", "High"),
            (Confidence::Medium, "\"medium\"", "Medium"),
            (Confidence::Low, "\"low\"", "Low"),
            (Confidence::Suspect, "\"suspect\"", "Suspect"),
        ] {
            assert_eq!(serde_json::to_string(&variant).unwrap(), wire);
            assert_eq!(serde_json::from_str::<Confidence>(wire).unwrap(), variant);
            assert_eq!(variant.to_string(), display);
        }
        // The client relies on this default: an old daemon's missing
        // locate.confidence decodes to the historic optimistic High.
        assert_eq!(Confidence::default(), Confidence::High);
    }

    #[test]
    fn provenance_vocabulary_and_unknown_fallback() {
        for (variant, wire, display) in [
            (Provenance::Api, "\"api\"", "api"),
            (Provenance::Cache, "\"cache\"", "cache"),
            (Provenance::NotFound, "\"not_found\"", "not_found"),
            (Provenance::Unresolved, "\"unresolved\"", "unresolved"),
        ] {
            assert_eq!(serde_json::to_string(&variant).unwrap(), wire);
            assert_eq!(serde_json::from_str::<Provenance>(wire).unwrap(), variant);
            assert_eq!(variant.to_string(), display);
        }
        // Forward-compat: an unrecognized future token decodes to Unknown
        // rather than failing the whole response (the client tolerance).
        assert_eq!(
            serde_json::from_str::<Provenance>("\"future_source\"").unwrap(),
            Provenance::Unknown
        );
    }

    #[test]
    fn db_status_vocabulary_and_unknown_fallback() {
        for (variant, wire, display) in [
            (DbStatus::Cached, "\"cached\"", "cached"),
            (DbStatus::Pending, "\"pending\"", "pending"),
            (DbStatus::NotFound, "\"not_found\"", "not_found"),
            (DbStatus::New, "\"new\"", "new"),
        ] {
            assert_eq!(serde_json::to_string(&variant).unwrap(), wire);
            assert_eq!(serde_json::from_str::<DbStatus>(wire).unwrap(), variant);
            assert_eq!(variant.to_string(), display);
        }
        assert_eq!(
            serde_json::from_str::<DbStatus>("\"future_variant\"").unwrap(),
            DbStatus::Unknown
        );
    }
}

/// Exact-byte guards proving the extraction preserved skip/emit/null/rename
/// behavior. These are what would break if a `skip_serializing_if` or a field
/// rename drifted.
mod wire_shape {
    use super::*;

    fn base_locate() -> LocateResponse {
        LocateResponse {
            ok: true,
            v: 1,
            lat: 55.5,
            lon: 12.25,
            accuracy_m: 42.5,
            sources: 3,
            cached: 2,
            visible: 4,
            stable: 3,
            address: None,
            stale: false,
            fallback: false,
            age_s: None,
            implied_speed_kmh: None,
            last_fix_age_secs: None,
            inlier_ratio: None,
            confidence: Confidence::High,
            suspect_reason: None,
            error: None,
        }
    }

    #[test]
    fn locate_omits_error_stale_fallback_and_absent_optionals_when_empty() {
        let json = serde_json::to_string(&base_locate()).unwrap();
        // error/stale/fallback and every None optional are skip_serializing_if;
        // a successful daemon response is byte-identical to the pre-extraction
        // wire, i.e. none of these keys appear.
        for absent in [
            "error",
            "stale",
            "fallback",
            "address",
            "age_s",
            "implied_speed_kmh",
            "last_fix_age_secs",
            "inlier_ratio",
            "suspect_reason",
        ] {
            assert!(!json.contains(absent), "unexpected `{absent}` in {json}");
        }
        // confidence is always emitted (the daemon sets it explicitly).
        assert!(json.contains("\"confidence\":\"high\""), "{json}");
    }

    #[test]
    fn locate_emits_stale_and_fallback_only_when_true() {
        let mut r = base_locate();
        r.stale = true;
        r.fallback = true;
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"stale\":true"), "{json}");
        assert!(json.contains("\"fallback\":true"), "{json}");
    }

    #[test]
    fn resolve_entry_uses_source_tag_and_null_optionals() {
        let json = serde_json::to_string(&ResolveResultEntry {
            bssid: "aa".to_string(),
            lat: None,
            lon: None,
            ssid: None,
            provenance: Provenance::Api,
        })
        .unwrap();
        // Wire tag is `source`, not `provenance`; lat/lon/ssid serialize as
        // null (no skip_serializing_if).
        assert_eq!(
            json,
            r#"{"bssid":"aa","lat":null,"lon":null,"ssid":null,"source":"api"}"#
        );
    }

    #[test]
    fn scan_emits_scan_age_ms_null_but_omits_scanned_at_and_error() {
        let json = serde_json::to_string(&ScanResponse {
            ok: true,
            v: 1,
            networks: vec![],
            scan_age_ms: None,
            scanned_at: None,
            error: None,
        })
        .unwrap();
        assert!(json.contains("\"scan_age_ms\":null"), "{json}");
        assert!(!json.contains("scanned_at"), "{json}");
        assert!(!json.contains("error"), "{json}");
    }

    #[test]
    fn history_fixes_omits_elided_and_error_when_absent() {
        // The daemon now serializes this shared type directly for raw history,
        // so `elided: None` must be omitted (not `null`), matching the
        // pre-extraction raw-history wire shape.
        let json = serde_json::to_string(&HistoryFixesResponse {
            ok: true,
            v: 1,
            range: HistoryRange {
                from_rfc3339: "a".to_string(),
                to_rfc3339: "b".to_string(),
            },
            fixes: vec![],
            elided: None,
            error: None,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"ok":true,"v":1,"range":{"from_rfc3339":"a","to_rfc3339":"b"},"fixes":[]}"#
        );
    }

    #[test]
    fn debug_bssid_emits_signal_null_when_absent() {
        let json = serde_json::to_string(&DebugBssid {
            bssid: "bb".to_string(),
            signal_dbm: None,
            seen: 2,
            needed: 3,
            is_stable: false,
            db_status: DbStatus::New,
        })
        .unwrap();
        assert!(json.contains("\"signal_dbm\":null"), "{json}");
    }
}

/// Client -> daemon request byte shapes. Requests are outbound-only (the client
/// serializes, the daemon parses with its own tagged enum), so these pin the
/// exact request bytes rather than round-tripping.
mod requests {
    use super::*;

    #[test]
    fn simple_and_locate_and_resolve_bytes() {
        assert_eq!(
            serde_json::to_string(&LocateRequest { cmd: "locate" }).unwrap(),
            r#"{"cmd":"locate"}"#
        );
        assert_eq!(
            serde_json::to_string(&SimpleRequest { cmd: "stats" }).unwrap(),
            r#"{"cmd":"stats"}"#
        );
        assert_eq!(
            serde_json::to_string(&ResolveRequest {
                cmd: "resolve",
                bssids: vec!["aa".to_string(), "bb".to_string()],
            })
            .unwrap(),
            r#"{"cmd":"resolve","bssids":["aa","bb"]}"#
        );
    }

    #[test]
    fn legacy_history_request_omits_absent_bounds() {
        assert_eq!(
            serde_json::to_string(&HistoryRequest {
                cmd: "history",
                range: Some("24h".to_string()),
                from: None,
                to: None,
            })
            .unwrap(),
            r#"{"cmd":"history","range":"24h"}"#
        );
    }

    #[test]
    fn raw_history_request_is_explicit_and_carries_the_cap() {
        assert_eq!(
            serde_json::to_string(&HistoryFixesRequest {
                cmd: "history",
                range: Some("24h".to_string()),
                from: None,
                to: None,
                raw: true,
                max_fixes: Some(1_234),
            })
            .unwrap(),
            r#"{"cmd":"history","range":"24h","raw":true,"max_fixes":1234}"#
        );
    }
}

/// Byte-exact strings hand-derived from the daemon emit path decode into the
/// typed responses (carried from the task-0152 cross-tests). Now that both
/// sides link ONE type these are technically self-checks, but they still guard
/// against an accidental field/attr change that would alter the wire.
mod daemon_fixtures {
    use super::*;

    #[track_caller]
    fn approx(actual: f64, expected: f64) {
        let tol = 1e-9 * expected.abs().max(1.0);
        assert!(
            (actual - expected).abs() <= tol,
            "float mismatch: actual={actual} expected={expected} tol={tol}"
        );
    }

    const LOCATE_FIXTURE: &str = r#"{"ok":true,"v":1,"lat":55.6761,"lon":12.5683,"accuracy_m":42.5,"sources":7,"cached":5,"visible":9,"stable":6,"address":"Copenhagen, Denmark","stale":true,"age_s":120,"implied_speed_kmh":13.5,"last_fix_age_secs":30,"inlier_ratio":0.75,"confidence":"medium","suspect_reason":"speed"}"#;
    const LOCATE_FALLBACK_FIXTURE: &str = r#"{"ok":true,"v":1,"lat":0.0,"lon":0.0,"accuracy_m":1000.0,"sources":2,"cached":2,"visible":2,"stable":2,"fallback":true,"confidence":"low"}"#;
    const RESOLVE_FIXTURE: &str = r#"{"ok":true,"v":1,"results":[{"bssid":"02:00:00:00:00:01","lat":55.1,"lon":12.2,"ssid":"cafe","source":"cache"},{"bssid":"02:00:00:00:00:02","lat":null,"lon":null,"ssid":null,"source":"not_found"}]}"#;
    const SCAN_FIXTURE: &str = r#"{"ok":true,"v":1,"networks":[{"bssid":"02:00:00:00:00:0a","ssid":"home","signal_dbm":-52,"channel":6}],"scan_age_ms":1500,"scanned_at":"2026-01-02T03:04:05Z"}"#;
    const STATS_FIXTURE: &str = r#"{"ok":true,"v":1,"cached_aps":100,"pending_aps":5,"not_found_aps":3,"db_size_bytes":40960,"api_calls_today":12,"api_counter_corruption_denials":3,"inflight":2,"db_write_failures":1,"db_read_failures":4,"autolocate_fixes_written":7,"orphan_tasks_aborted":0}"#;
    const DEBUG_FIXTURE: &str = r#"{"ok":true,"v":1,"daemon_rev":"abc1234","scan_age_ms":800,"samples_in_buffer":4,"visible":3,"stable":2,"bssids":[{"bssid":"02:00:00:00:00:aa","signal_dbm":-48,"seen":5,"needed":3,"is_stable":true,"db_status":"cached"},{"bssid":"02:00:00:00:00:bb","signal_dbm":null,"seen":2,"needed":3,"is_stable":false,"db_status":"new"}]}"#;
    const HISTORY_FIXTURE: &str = r#"{"ok":true,"v":1,"from":"2026-01-01T00:00:00Z","to":"2026-01-02T00:00:00Z","segments":[{"start_rfc3339":"2026-01-01T08:00:00Z","end_rfc3339":"2026-01-01T09:30:00Z","duration_secs":5400,"centroid_lat":55.6761,"centroid_lon":12.5683,"mean_accuracy_m":35.0,"n_fixes":42}]}"#;
    const VERSION_FIXTURE: &str = r#"{"ok":true,"v":1,"version":"0.1.0","git_rev":"deadbeef"}"#;
    const ERROR_FIXTURE: &str = r#"{"ok":false,"v":1,"error":"unknown command"}"#;

    #[test]
    fn locate_fixture_populates_every_field_and_absent_fallback_defaults_false() {
        let r: LocateResponse = serde_json::from_str(LOCATE_FIXTURE).unwrap();
        assert!(r.ok);
        approx(r.lat, 55.6761);
        approx(r.lon, 12.5683);
        approx(r.accuracy_m, 42.5);
        assert_eq!(r.sources, 7);
        assert_eq!(r.address.as_deref(), Some("Copenhagen, Denmark"));
        assert!(r.stale);
        assert!(!r.fallback, "absent fallback must default to false");
        assert_eq!(r.age_s, Some(120));
        assert_eq!(r.confidence, Confidence::Medium);
        assert_eq!(r.suspect_reason.as_deref(), Some("speed"));
        assert!(r.error.is_none());
    }

    #[test]
    fn locate_fallback_fixture_decodes_true() {
        let r: LocateResponse = serde_json::from_str(LOCATE_FALLBACK_FIXTURE).unwrap();
        assert!(r.fallback);
        approx(r.accuracy_m, 1000.0);
        assert!(!r.stale);
    }

    #[test]
    fn resolve_fixture_populates_entries_and_provenance() {
        let r: ResolveResponse = serde_json::from_str(RESOLVE_FIXTURE).unwrap();
        assert_eq!(r.results.len(), 2);
        assert_eq!(r.results[0].provenance, Provenance::Cache);
        assert!(r.results[1].lat.is_none());
        assert_eq!(r.results[1].provenance, Provenance::NotFound);
    }

    #[test]
    fn scan_stats_debug_history_version_fixtures_decode() {
        let s: ScanResponse = serde_json::from_str(SCAN_FIXTURE).unwrap();
        assert_eq!(s.networks[0].signal_dbm, -52);
        assert_eq!(s.scan_age_ms, Some(1500));

        let st: StatsResponse = serde_json::from_str(STATS_FIXTURE).unwrap();
        assert_eq!(st.cached_aps, 100);
        assert_eq!(st.orphan_tasks_aborted, 0);

        let d: DebugResponse = serde_json::from_str(DEBUG_FIXTURE).unwrap();
        assert_eq!(d.daemon_rev.as_deref(), Some("abc1234"));
        assert_eq!(d.bssids[1].signal_dbm, None);
        assert_eq!(d.bssids[1].db_status, DbStatus::New);

        let h: HistoryResponse = serde_json::from_str(HISTORY_FIXTURE).unwrap();
        assert_eq!(h.segments.len(), 1);
        assert_eq!(h.segments[0].n_fixes, 42);

        let v: VersionResponse = serde_json::from_str(VERSION_FIXTURE).unwrap();
        assert_eq!(v.version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn legacy_stats_defaults_missing_counters_to_zero() {
        let json = r#"{"ok":true,"v":1,"cached_aps":101,"pending_aps":102,"not_found_aps":103,"db_size_bytes":104,"api_calls_today":105,"inflight":106,"db_write_failures":107}"#;
        let st: StatsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(st.api_counter_corruption_denials, 0);
        assert_eq!(st.db_read_failures, 0);
        assert_eq!(st.autolocate_fixes_written, 0);
        assert_eq!(st.orphan_tasks_aborted, 0);
    }

    #[test]
    fn error_envelope_decodes_into_any_typed_response() {
        let locate: LocateResponse = serde_json::from_str(ERROR_FIXTURE).unwrap();
        assert!(!locate.ok);
        assert_eq!(locate.error.as_deref(), Some("unknown command"));

        let scan: ScanResponse = serde_json::from_str(ERROR_FIXTURE).unwrap();
        assert!(!scan.ok);
        assert!(scan.networks.is_empty());
    }

    #[test]
    fn a_daemon_rename_is_caught_by_decode_or_by_value() {
        // Required (non-defaulted) field rename -> hard decode error.
        let renamed_required = RESOLVE_FIXTURE.replace(
            r#""bssid":"02:00:00:00:00:01""#,
            r#""station":"02:00:00:00:00:01""#,
        );
        assert!(serde_json::from_str::<ResolveResponse>(&renamed_required).is_err());

        // Defaulted scalar rename -> decodes but reads the zero default.
        let renamed_defaulted =
            LOCATE_FIXTURE.replace(r#""accuracy_m":42.5"#, r#""accuracy_meters":42.5"#);
        let r: LocateResponse = serde_json::from_str(&renamed_defaulted).unwrap();
        assert_eq!(r.accuracy_m, 0.0);
    }
}
