---
id: TASK-0152
title: 'Test: wire-format daemon-emit -> client-decode golden cross-test'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:29'
updated_date: '2026-07-08 19:46'
labels:
  - tests
  - wire-format
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P0-5. server.rs:996-1085 and lib.rs:456-535 both test their own enums in isolation. No test runs daemon-produced JSON through the client deserializer end-to-end. Daemon renaming accuracy_m or adding a non-Option field with no serde(default) on client wouldn't be caught. Also: client lib.rs:339 reads exactly one line — multi-line response (someone uses serde_json::to_string_pretty by mistake) would silently truncate.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 In whereami-client tests: embed one byte-exact daemon response per command (locate, scan, stats, debug, history, version, resolve, error) as &str fixtures
- [x] #2 serde_json::from_slice each into the typed Response; assert all fields populate as expected
- [x] #3 Two-line framing test: feed 'response_line1\nrest' to the client reader; assert it reads exactly the first line (catches multi-line serialization mistakes)
- [ ] #4 Optional: a test that runs the actual daemon-side serialize -> client-side deserialize round-trip for one struct per command
- [x] #5 Composes cleanly with task-0114 (whereami-protocol crate) when that lands
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Read daemon response structs (server.rs) + serde attrs + existing daemon wire tests as fixture source of truth.
2. Add mod daemon_wire_cross_tests to whereami-client/src/lib.rs: byte-exact daemon-emit &str fixtures for locate/scan/stats/debug/history/version/resolve/error, hand-derived from daemon struct declaration order + serde attrs.
3. serde_json::from_slice each into typed client Response; assert real field values (floats with tolerance to avoid serde_json non-roundtrip ULP flake).
4. Framing test: real TcpListener writes <json>\n<garbage>; WhereAmIClient::tcp raw_command must return exactly first line and decode it.
5. AC#4 round-trip: SKIP (client cannot depend on whereamid; blocked on task-0114 protocol crate) unless cleanly reachable.
6. Gate: cargo test -p whereami-client (x2), just e2e, clippy -D warnings, fmt --check. Report bite analysis re serde(default).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from task-0245: serde_json's DEFAULT float parser is NOT correctly-rounded (the `float_roundtrip` feature is off in this workspace), so `serde_json::from_str(...).as_f64()` of a serialized f64 can land 1 ULP off the exact value. A golden cross-test that emits a float on the daemon side and decodes it on the client side must compare floats with a TOLERANCE, not exact `assert_eq!` — otherwise it is ULP-flaky. See `assert_speed_matches` in whereamid/src/server.rs tests for the pattern (relative ~1e-9).

Implemented daemon_wire_cross_tests in whereami-client/src/lib.rs (commit 23be024).

What landed:
- 8 byte-exact daemon-emit fixtures (locate/resolve/scan/stats/debug/history/version/error) hand-derived from the daemon Serialize structs in whereamid/src/server.rs (LocateResponse:565, ResolveResult:632, ScanResponse/NetworkInfo:645/655, StatsResponse:663, DebugResponse/DebugBssid:2410/2398, HistoryResponse:2532 + history::Segment, VersionResponse:2514, error_json:710), in declaration order, respecting each skip_serializing_if.
- Cross-checked value shapes against the daemon own exact-JSON wire tests: stats_wire_reports_corruption_denials_... (server.rs:3245) and scan_and_debug_option_fields_pin_null_vs_omitted_wire_shape (server.rs:6507) — the latter confirms scan_age_ms/signal_dbm/ssid/channel serialize as JSON null while scanned_at is omitted.
- AC#1/#2: from_slice each fixture into the typed client Response, assert every field (floats via relative tolerance, not assert_eq!, because serde_json float_roundtrip is OFF here — carried from task-0245).
- AC#3 framing: real loopback TcpListener writes "<json>\n<garbage>"; WhereAmIClient::tcp raw_command drives the actual exchange/BufReader::read_line path and must return exactly the first line and decode it.
- Bite test: renaming a NON-defaulted field (bssid) fails decode; renaming a serde(default) scalar (accuracy_m) still decodes but value assertion catches it.

GATE (all green): cargo test -p whereami-client lib = 49 passed / 0 failed (10 new), run twice 10/10; full -p suite green; just e2e exit 0; clippy --all-targets -D warnings clean (fixed doc_lazy_continuation); cargo fmt --check clean.

HONEST LIMITS / bite scope: the client puts #[serde(default)] on nearly every top-level scalar, so a daemon rename of those does NOT fail decode — only the VALUE assertion bites. Genuine hard-decode protection is limited to non-defaulted fields (bssid, signal_dbm, provenance, all HistorySegment/HistoryFix fields, ok, v). Fixtures are hand-written, not machine-generated from whereamid, so they can silently rot if the daemon struct changes and nobody updates them — the real fix is a shared protocol crate.

AC#4 (daemon-serialize -> client-deserialize round-trip): SKIPPED. whereami-client does not (and must not) depend on whereamid; a true round-trip needs the shared whereami-protocol crate tracked by task-0114. Composes cleanly with it (AC#5).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added an end-to-end daemon-emit -> client-decode wire cross-test (commit 23be024, whereami-client/src/lib.rs, TEST-ONLY). Previously the daemon and client tested their wire types in ISOLATION; nothing ran daemon-produced JSON through the client deserializer, so a daemon field rename or a new non-Option field without client serde(default) would go uncaught.

New #[cfg(test)] mod daemon_wire_cross_tests: 8 byte-exact daemon-response fixtures (locate/resolve/scan/stats/debug/history/version/error), each from_slice-d into the typed client Response with real per-field value assertions (AC#1/#2); plus a real loopback-TcpListener framing test that writes "<json>\n<garbage>" and drives the actual WhereAmIClient exchange/BufReader::read_line path, asserting exactly the first line is read + decoded (AC#3).

FIXTURE AUTHENTICITY (the crux): every fixture mirrors the DAEMON's #[derive(Serialize)] structs read from whereamid/src/server.rs (LocateResponse/ResolveResult/ScanResponse/StatsResponse/DebugResponse/HistoryResponse/VersionResponse/error_json) in serde declaration order + skip_serializing_if semantics, cross-checked against the daemon's own exact-JSON wire tests (stats_wire_reports_..., scan_and_debug_option_fields_pin_null_vs_omitted_wire_shape) — derived from daemon output, not client expectations, so it is not circular.

HONEST BITE LIMITATION (explicit, with a dedicated test a_daemon_rename_is_caught_by_decode_or_by_value): non-defaulted client fields (bssid/signal_dbm/provenance/ok/v/all HistorySegment+HistoryFix fields) fail from_slice OUTRIGHT on a daemon rename; #[serde(default)] fields (most locate/stats scalars incl. accuracy_m) decode fine and are caught ONLY by the value assertion (reads zero default). Float asserts use 1e-9 relative tolerance (serde_json here lacks float_roundtrip — carried from task-0245).

AC#4 (live daemon-serialize->client-deserialize round-trip) SKIPPED — needs the daemon structs, which live in the separate whereamid crate the client neither depends on nor should; blocked pending the shared whereami-protocol crate (task-0114). AC#5: the fixtures/structure compose cleanly with 0114 when it lands.

Verification: test-only diff (git-confirmed 0 production lines), orchestrator-verified: whereami-client lib 49/49 (10 new, pass twice), clippy -D warnings clean, fmt clean, just e2e exit 0.

Durable limit: fixtures are hand-written, so they can silently rot if a daemon struct changes and nobody updates them; the durable fix is the shared protocol crate (task-0114). The module doc records the exact daemon source lines to re-check against.
<!-- SECTION:FINAL_SUMMARY:END -->
