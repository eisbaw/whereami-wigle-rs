---
id: TASK-0227
title: Fail-fast on non-finite lat/lon entering trilaterate (cache-read guard)
status: To Do
assignee: []
created_date: '2026-07-02 06:34'
labels:
  - correctness
  - math
dependencies:
  - TASK-0185
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from the task-0185 review. The median() NaN-proofing (total_cmp) removed the panic, but a NaN/Inf coordinate read from a cached aps row still flows through to_unit_vec -> NaN vector -> the mag>0 guards are false-on-NaN -> from_unit_vec returns (NaN, NaN) as an Ok Position. SQLite REAL columns can hold NaN. task-0185 only constrained finite INPUTS to haversine; non-finite coords entering the pipeline are only symptom-patched. Ref whereamid/src/trilaterate.rs (to_unit_vec/from_unit_vec, entry), resolver.rs lookup_cached / db.rs aps read.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 non-finite lat/lon read for an AP is skipped (not fed to trilaterate) with a warn! naming the bssid
- [ ] #2 trilaterate never returns a Position with non-finite lat/lon for any cached-row input
- [ ] #3 a unit/proptest test injects a NaN-coord AP and asserts it is excluded, not returned
<!-- AC:END -->
