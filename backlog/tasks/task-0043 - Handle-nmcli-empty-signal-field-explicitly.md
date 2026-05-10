---
id: TASK-0043
title: Handle nmcli empty signal field explicitly
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 05:39'
updated_date: '2026-05-10 07:00'
labels:
  - scanner
  - data-quality
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
scanner.rs nmcli parsing falls back to signal=0% on a missing/malformed signal field, then converts via the linear formula to signal_dbm = -90. This is indistinguishable from a real weak AP and feeds garbage weights into trilateration. Either skip rows with no signal or model signal as Option<i32>. Also: the linear conversion (-90 + signal_pct*60/100) is documented to be inaccurate vs. iw's dBm; consider preferring iw exclusively when accuracy matters or document the limitation in PRD.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Empty/missing nmcli signal field produces a skipped row OR a None signal, not a fake -90 dBm
- [x] #2 Existing parser tests cover the 'empty signal' input shape
- [x] #3 PRD or scanner.rs comment documents the nmcli vs iw signal accuracy limitation
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Skip rows with empty/malformed signal field instead of falling back to 0% (fake -90 dBm)
2. Add a comment in the parser noting nmcli's pct→dBm conversion is a coarse approximation vs iw's authoritative dBm
3. Add a test for the empty-signal row → not emitted
4. Run fuzz_nmcli_parser 30s smoke
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented skip-on-missing: rows where fields[2].trim().parse() fails are dropped. Two new tests: test_parse_nmcli_skips_empty_signal (empty signal field) and test_parse_nmcli_skips_garbage_signal (non-numeric). Added an inline comment in the parser noting that nmcli's pct→dBm linear approximation is coarse and driver-dependent vs iw's authoritative dBm — positions trilaterated from nmcli scans alone will have systematically biased weights.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
nmcli parser now skips rows with empty/malformed signal fields instead of coercing to 0% (which the linear conversion mapped to a fake -90 dBm, indistinguishable from a real weak AP and poisoning trilateration weights). Comment in parser documents the coarseness of nmcli's pct→dBm vs iw's authoritative dBm. Tests cover empty and garbage cases. fuzz_nmcli_parser 30s clean (53,021 runs).
<!-- SECTION:FINAL_SUMMARY:END -->
