---
id: TASK-0148
title: 'Test: apple::decode_response golden positive-path coverage'
status: To Do
assignee: []
created_date: '2026-05-20 21:29'
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
- [ ] #1 Capture one real Apple WPS response (or hand-build a valid protobuf with 2-3 WifiDevice entries each containing bssid + Location lat/lon at known coordinates)
- [ ] #2 Embed the bytes as a &[u8] literal in apple.rs tests
- [ ] #3 Test asserts decoded ApInfo: BSSID set, lat/lon at expected ±epsilon, requested_set filter works (extra BSSIDs in response dropped)
- [ ] #4 Bonus: round-trip test that encode_request(['AA:BB:CC:DD:EE:FF']) -> simulated server -> decode_response gives back the same BSSID with the response's lat/lon
<!-- AC:END -->
