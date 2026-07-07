---
id: TASK-0152
title: 'Test: wire-format daemon-emit -> client-decode golden cross-test'
status: To Do
assignee: []
created_date: '2026-05-20 21:29'
updated_date: '2026-07-07 23:09'
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
- [ ] #1 In whereami-client tests: embed one byte-exact daemon response per command (locate, scan, stats, debug, history, version, resolve, error) as &str fixtures
- [ ] #2 serde_json::from_slice each into the typed Response; assert all fields populate as expected
- [ ] #3 Two-line framing test: feed 'response_line1\nrest' to the client reader; assert it reads exactly the first line (catches multi-line serialization mistakes)
- [ ] #4 Optional: a test that runs the actual daemon-side serialize -> client-side deserialize round-trip for one struct per command
- [ ] #5 Composes cleanly with task-0114 (whereami-protocol crate) when that lands
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from task-0245: serde_json's DEFAULT float parser is NOT correctly-rounded (the `float_roundtrip` feature is off in this workspace), so `serde_json::from_str(...).as_f64()` of a serialized f64 can land 1 ULP off the exact value. A golden cross-test that emits a float on the daemon side and decodes it on the client side must compare floats with a TOLERANCE, not exact `assert_eq!` — otherwise it is ULP-flaky. See `assert_speed_matches` in whereamid/src/server.rs tests for the pattern (relative ~1e-9).
<!-- SECTION:NOTES:END -->
