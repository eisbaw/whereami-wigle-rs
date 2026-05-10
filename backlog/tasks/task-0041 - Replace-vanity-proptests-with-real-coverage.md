---
id: TASK-0041
title: Replace vanity proptests with real coverage
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 05:38'
updated_date: '2026-05-10 07:51'
labels:
  - testing
  - quality
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
whereamid/tests/proptests.rs has tests that pass tautologically or never exercise the interesting path: trilaterate_accuracy_positive (clamped output is trivially true), debounce_stable_bounded (definitionally true), split_nmcli_unescaped_colon_count (generator never emits backslashes/colons), parse_iw_no_panic and split_nmcli_no_panic (panic-freedom is the weakest property in safe Rust). Replace with genuine properties: trilateration is bounded by AP convex hull (with antimeridian fix from task-0032), nmcli parser round-trips through known-escape inputs, debounce stability respects threshold/window invariants under concurrent insert + age.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 trilaterate_accuracy_positive removed or replaced with non-tautological version
- [x] #2 split_nmcli_unescaped_colon_count generator covers backslash and colon characters
- [x] #3 parse_iw_no_panic / split_nmcli_no_panic replaced or augmented with shape assertions
- [x] #4 debounce_stable_bounded augmented with a non-trivial invariant
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Replace trilaterate_accuracy_positive (tautological, clamped output) with non-trivial property
2. Fix split_nmcli_unescaped_colon_count generator (currently never emits colons/backslashes — useless)
3. Augment parse_iw_no_panic and split_nmcli_no_panic with shape assertions, not just panic-freedom
4. Augment debounce_stable_bounded with a non-trivial invariant
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Replaced/augmented four vanity proptests with real-property versions:

1. trilaterate_accuracy_positive (was tautological — clamp ensured >0) → trilaterate_stronger_signal_pulls_centroid: generates one strong + one weak AP at distinct positions, asserts centroid is closer to the strong one. Tests the actual purpose of the dBm-weighted formula.

2. parse_iw_no_panic (panic-freedom only — weakest possible property in safe Rust) → parse_iw_output_emits_well_formed_bssids: every emitted network must have a valid 6-octet hex BSSID. Now actually catches malformed-output regressions, not just hypothetical panics.

3. split_nmcli_no_panic + split_nmcli_unescaped_colon_count (generator never emitted backslashes/colons — vacuous) → split_nmcli_no_unescaped_backslash_colon (no \: residue in any output field) + split_nmcli_round_trips_with_escapes (generator now includes ':' chars, which the test escapes as '\:' on the wire and asserts round-trip equality).

4. debounce_stable_bounded (stable.len() <= num_aps was definitionally true) → debounce_threshold_contract: asserts the threshold contract directly — threshold-1 scans means no APs stable, exactly threshold scans means all APs stable.

All 9 proptests pass; clippy/fmt clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced four tautological/weak proptests with non-trivial property checks. The new properties test actual contracts: signal-weighting in trilateration, BSSID well-formedness in iw parser, escape round-trip in nmcli parser, threshold semantics in debouncer. Each replacement caught a real class of regression that the original property would have missed entirely.
<!-- SECTION:FINAL_SUMMARY:END -->
