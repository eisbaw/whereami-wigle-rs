---
id: TASK-0148
title: 'Test: apple::decode_response golden positive-path coverage'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:29'
updated_date: '2026-07-08 19:05'
labels:
  - tests
  - apple
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P0-1. apple.rs:148 decode_response has NO byte-exact positive-path test. apple.rs encoder tests cover 410-507 but not decoder. fuzz_apple_decode only checks panic-freedom; Kani only panic-freedom. A refactor mixing field 1/2, dropping the requested_set filter (task-0049 regression), or losing the <-179 threshold would pass every existing test.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Capture one real Apple WPS response (or hand-build a valid protobuf with 2-3 WifiDevice entries each containing bssid + Location lat/lon at known coordinates)
- [x] #2 Embed the bytes as a &[u8] literal in apple.rs tests
- [x] #3 Test asserts decoded ApInfo: BSSID set, lat/lon at expected ±epsilon, requested_set filter works (extra BSSIDs in response dropped)
- [x] #4 Bonus: round-trip test that encode_request(['AA:BB:CC:DD:EE:FF']) -> simulated server -> decode_response gives back the same BSSID with the response's lat/lon
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Reuse existing test builders (encode_location_msg, encode_wifi_device_msg, encode_bytes, scaled) in mod tests.
2. Add golden decode_response test: build response body with 3 WifiDevice entries at known coords, request only 2 of them, assert exactly-2 returned with normalized BSSID + lat/lon within 1e-4, unrequested dropped (pins task-0049 filter).
3. Add round-trip-style test: single BSSID response -> decode_response returns it with the response coords.
4. Verify: cargo test -p whereamid apple (lib+bin), just e2e, clippy -D warnings, fmt --check. Run new tests twice.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Added 3 decode_response tests in apple.rs mod tests, reusing existing builders (encode_location_msg/encode_wifi_device_msg/encode_bytes/scaled).
- Fixed-point scale: 1e-8 (decoder does lt as f64 * 1e-8, apple.rs:234). Reused the existing scaled() helper: (deg / 1e-8).round() as i64. Verified expected coords decode within 1e-4.
- Gate: cargo test -p whereamid apple = lib 18 passed / bin 20 passed (both targets, run twice, identical). clippy --all-targets -D warnings clean. fmt --check clean. just e2e all green.
- Filter pin: decode_response_golden_filters_to_requested_bssids asserts aps.len()==2 and find(#3).is_none(); removing requested_set.contains would leak BSSID #3 and fail both.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added byte-exact positive-path coverage for apple::decode_response, which previously had only panic-freedom tests (fuzz/Kani) — a refactor breaking the requested_set filter (task-0049 regression), swapping field 1/2, or losing the position sentinel would have passed every existing test.

Changes (commit 03f01ae, whereamid/src/apple.rs, TEST-ONLY): 3 tests + a push_wifi_device helper in mod tests:
- decode_response_golden_filters_to_requested_bssids: hand-builds a response with 3 WifiDevice entries at known coords (Copenhagen/Cupertino/Sydney), requests only 2, asserts exactly those 2 decode with lat/lon within +-1e-4 AND the unrequested Sydney BSSID is DROPPED (pins the task-0049 requested_set filter).
- decode_response_round_trip_single_bssid (AC#4 bonus, response->decode direction).
- decode_response_matches_bssid_case_insensitively (normalization).
Fixed-point scale 1e-8 (decoder does `lt as f64 * 1e-8`, apple.rs:234); reused the existing `scaled(deg)` test helper so encoded input round-trips exactly.

Verification: test-only diff (git-confirmed no production lines changed), verified by orchestrator: apple lib 18 / bin 20 (stable, both dual-target binaries), clippy -D warnings + fmt clean, e2e green. FILTER BITE MUTATION-CONFIRMED by orchestrator: neutering `if requested_set.contains(&ap.bssid)` to always-push makes decode_response_golden_filters_to_requested_bssids FAIL (left 3, right 2), then restored — so the test genuinely guards the filter. No follow-ups.

Judgment call (AC#2): bytes are built deterministically via the existing protobuf encoders rather than pasted as an opaque hex literal — more readable/self-documenting and equally valid per AC#1s "or hand-build" path. Honest limit: no CAPTURED real Apple response is pinned (would need live capture), so a never-modeled wire quirk is out of scope.
<!-- SECTION:FINAL_SUMMARY:END -->
