---
id: TASK-0179
title: 'Expand fuzz coverage: parsers, decoders, edge-case generators'
status: To Do
assignee: []
created_date: '2026-05-20 21:36'
labels:
  - tooling
  - fuzz
  - tests
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Today's fuzz targets (5): fuzz_iw_parser, fuzz_nmcli_parser, fuzz_apple_decode, fuzz_apple_encode, fuzz_trilaterate. Coverage gaps identified by Scout review and codebase audit. Goal: every parser + every decoder + every public API surface exposed to untrusted input is fuzzed.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add fuzz_nominatim_decode for the Nominatim JSON response shape (untrusted external input)
- [ ] #2 Add fuzz_wigle_decode for the WiGLE JSON response shape (untrusted external input)
- [ ] #3 Add fuzz_parse_range for history.rs::parse_range (covers UTF-8 panic class — task-0048 incident)
- [ ] #4 Add fuzz_normalize_bssid + fuzz_split_nmcli_fields (currently only proptests)
- [ ] #5 Add fuzz_segment_fixes feeding random Vec<FixRow> into history segmentation
- [ ] #6 Add fuzz_resolve_request_decode for the daemon's JSON request shape (incoming /resolve, /locate, etc.)
- [ ] #7 All new fuzz targets compile + run 15s each clean (no panics) on baseline
- [ ] #8 Justfile fuzz-each (task-0171) includes the new targets automatically
- [ ] #9 Composes with task-0163 (CI runs fuzz-build) and task-0036 (Apple WPS encoder fuzz)
<!-- AC:END -->
