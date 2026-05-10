---
id: TASK-0041
title: Replace vanity proptests with real coverage
status: To Do
assignee: []
created_date: '2026-05-10 05:38'
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
- [ ] #1 trilaterate_accuracy_positive removed or replaced with non-tautological version
- [ ] #2 split_nmcli_unescaped_colon_count generator covers backslash and colon characters
- [ ] #3 parse_iw_no_panic / split_nmcli_no_panic replaced or augmented with shape assertions
- [ ] #4 debounce_stable_bounded augmented with a non-trivial invariant
<!-- AC:END -->
