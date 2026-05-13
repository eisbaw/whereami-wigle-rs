---
id: TASK-0086
title: Derive Copy on PositionedAp
status: Done
assignee:
  - '@mpedersen'
created_date: '2026-05-13 21:19'
updated_date: '2026-05-13 21:32'
labels:
  - hygiene
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PositionedAp in trilaterate.rs has all-Copy fields (f64, f64, Option<i32>) but only derives Clone. Adding Copy removes ~3 explicit .clone() calls in drop_isolated/filter_outliers and lets the struct be passed by value freely. This is aesthetic, not a perf win (Clone for this struct is already a memcpy). Caveat: PositionedAp is pub, so adding Copy is a soft promise to consumers; if a future field is non-Copy (e.g. String SSID, owning Source enum), reverting forces every by-value call site to clone or take a reference. Worth doing only if there is high confidence the struct shape will stay all-Copy.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 PositionedAp derives Copy in addition to Clone, Debug
- [x] #2 Explicit .clone() calls on PositionedAp removed where the borrow checker now permits
- [x] #3 All trilaterate tests and proptests still pass
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add Copy to PositionedAp derive\n2. Remove .clone() / .cloned() calls in trilaterate.rs and proptests where borrow checker allows\n3. Run cargo test/clippy/fmt
<!-- SECTION:PLAN:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Derive Copy on PositionedAp (all-Copy fields: f64, f64, Option<i32>) and remove the now-redundant explicit clone calls.

Changes:
- trilaterate.rs: PositionedAp now derives Copy in addition to Clone, Debug. Doc comment notes the soft-promise caveat (a future non-Copy field would force every by-value call site to clone or take a reference).
- trilaterate.rs: removed five `aps[i].clone()` calls inside filter_outliers (the indexed PositionedAp is now produced by Copy).
- proptests.rs: dropped two `.clone()` calls on PositionedAp in the trilaterate_stronger_signal_pulls_centroid test.

No clone calls remain on PositionedAp in the codebase. Other clones flagged by grep are on different types (String, FixRow.at_rfc3339, address strings, etc.) and are unrelated.

Tests: cargo test workspace (177 pass, same as task-0085 baseline), cargo clippy --all-targets -- -D warnings clean, cargo fmt --check clean. This is a pure refactor; no behavior change.
<!-- SECTION:FINAL_SUMMARY:END -->
