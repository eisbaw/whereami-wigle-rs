---
id: TASK-0146
title: Split trilaterate.rs into submodules (1245 lines)
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:28'
updated_date: '2026-07-09 03:43'
labels:
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/trilaterate.rs: 1245 lines, near practical reading-comfort ceiling. Tasks 0055/0056 split db.rs and server.rs; trilaterate.rs is next. Natural split: centroid (weighted spherical mean), outliers (drop_isolated + filter_outliers), median (Weiszfeld + GeoMedianResult).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Split into trilaterate/mod.rs (re-exports) + centroid.rs + outliers.rs + median.rs (or chosen-naming)
- [x] #2 Tests stay with their modules (each submodule has its own #[cfg(test)] mod)
- [x] #3 Kani harnesses move accordingly
- [x] #4 Existing API unchanged: pub use trilaterate::*; from lib.rs/main.rs continues to work
- [x] #5 Composes with task-0124 (rename to weighted_centroid) — coordinate
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Read trilaterate.rs (1979 lines) — done. No kani harnesses present.
2. Create trilaterate/ dir with mod.rs, centroid.rs, outliers.rs, median.rs.
3. centroid.rs: to_unit_vec, from_unit_vec (pub(crate)), ap_consensus_score (pub) + consensus consts + 4 consensus tests.
4. median.rs: GeoMedianResult, CancellableGeoMedianResult, Display, geometric_median{,_detailed,_detailed_cancellable} (pub(crate)), median helper (pub(crate)), WEISZFELD_* consts (MAX_ITERS pub(crate)) + 9 median tests.
5. outliers.rs: filter_outliers (pub), drop_isolated + stage consts + 7 outlier tests.
6. mod.rs: PositionedAp, Position, vote_weight, missing_signal_dbm, finite_position, trilaterate, estimate_accuracy, DBM/NOMINAL consts + re-exports + 19 orchestrator/spanning tests.
7. Delete old trilaterate.rs. Adjust only use/visibility — byte-identical bodies.
8. Gate: nix develop -c just e2e + cargo fmt --check. Verify test count unchanged (39 trilaterate, ~849 workspace).
9. Commit code only, no attribution. Leave In Progress.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented split (commit 7cf5c6f). trilaterate.rs (1979 lines) -> trilaterate/{mod,centroid,median,outliers}.rs.

- centroid.rs: to_unit_vec/from_unit_vec (pub(crate)), ap_consensus_score (pub) + 4 consensus tests.
- median.rs: GeoMedianResult/CancellableGeoMedianResult + Display, geometric_median{,_detailed,_detailed_cancellable}, median helper, WEISZFELD_* consts + 9 tests.
- outliers.rs: filter_outliers (pub), drop_isolated + stage consts + 7 tests.
- mod.rs: PositionedAp/Position, vote_weight/missing_signal_dbm/finite_position/estimate_accuracy, trilaterate orchestrator, re-exports + 19 tests.

Zero logic change: normalized sorted-line diff vs HEAD shows ONLY structural deltas (3x extra }/#[cfg(test)]/mod tests from 1->4 test modules, re-export block lines, and pub(crate) added to WEISZFELD_MAX_ITERS + median + median-API re-exports). No body/algorithm/constant/signature change.

Visibility: WEISZFELD_MAX_ITERS, median, geometric_median, geometric_median_detailed_cancellable, CancellableGeoMedianResult, GeoMedianResult -> pub(crate); re-exported at mod root to preserve crate::trilaterate::X for history.rs (binary-only), hence #[allow(unused_imports)] on that group (mirrors pre-existing #[allow(dead_code)] on geometric_median).

No #[cfg(kani)] harnesses existed in this file.

Gate: nix develop -c just e2e PASSED = 849 passed / 0 failed (baseline 849 = equal), clippy --all-targets -D warnings clean, cargo fmt --check clean. trilaterate tests: 39 (4+9+7+19), unchanged. whereamid/fuzz cargo check clean. server.rs/history.rs/proptests/fuzz all resolve.

AC#5: task-0124 weighted_centroid rename deliberately NOT performed here; split leaves it clean to follow.

Left In Progress for review gate; ACs not checked per workflow.

## Review gate (orchestrator, independent) — GO/GO, no hardening needed
- QA (2x): 849/849 == baseline (NO test dropped in the split); trilaterate submodule tests sum to 39 (mod 19 + median 9 + outliers 7 + centroid 4); clippy 0 -D warnings; fmt clean; fuzz crate checks; zero flake.
- Architecture: ZERO logic change CONFIRMED via byte-identical in-order diff of 8 load-bearing fns (trilaterate 116/116, filter_outliers 101/101, drop_isolated 15/15, Weiszfeld geometric_median_detailed_cancellable 95/95, to_unit_vec/from_unit_vec, vote_weight, estimate_accuracy). ALL 10 constants unchanged (WEISZFELD_MAX_ITERS=50, NEIGHBOR_RADIUS_M=2000, STAGE2_*, DBM_MIN/MAX, NOMINAL_MISSING_DBM, CONSENSUS_*, MIN_CONSENSUS_N); only edit = pub(crate) on WEISZFELD_MAX_ITERS + median-API re-export. No test assertion changed (whole-file set-equality diff: zero old-only substantive lines). External API preserved (mod.rs re-exports cover server.rs/history.rs/proptests/fuzz). #[allow(unused_imports)] on the median re-export group is genuine (bin/test-only consumers vs lib -D warnings), mirrors the pre-existing #[allow(dead_code)] on geometric_median.
- LOW (not fixed, non-defect): a breadcrumb comment (task-0081 haversine_m moved to crate::geo) was dropped; the `use crate::geo::haversine_m` import is self-documenting so zero impact.
- AC#5: the task-0124 weighted_centroid rename was NOT done here (separate task); the split leaves it clean to follow.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Split the 1979-line whereamid/src/trilaterate.rs into a trilaterate/ module directory for readability. Pure code movement, zero behavior change.

Layout:
- centroid.rs: to_unit_vec/from_unit_vec (unit-sphere helpers) + ap_consensus_score + consensus consts (4 tests).
- median.rs: GeoMedianResult/CancellableGeoMedianResult, the geometric_median family (Weiszfeld) + scalar median + WEISZFELD_* consts (9 tests).
- outliers.rs: filter_outliers + drop_isolated + stage consts (7 tests).
- mod.rs: PositionedAp/Position, trilaterate orchestrator, vote_weight/missing_signal_dbm/finite_position/estimate_accuracy, DBM/consensus consts, and re-exports (19 tests).

External API unchanged: mod.rs re-exports everything callers use (server.rs, history.rs, proptests, fuzz); crate::trilaterate::X all resolve. Only visibility bumps (pub(crate) on cross-submodule helpers) + use-path adjustments; every moved function body is byte-identical (reviewer diffed 8 load-bearing fns + all 10 constants).

Tests: 849 workspace == pre-split baseline (no test dropped); 39 trilaterate tests partitioned into the 4 modules with unchanged assertions. Two independent gate runs zero flake, clippy -D warnings clean, fmt clean, fuzz crate checks.

Scope: the task-0124 weighted_centroid rename (AC#5) was intentionally NOT done — separate task; the split is left clean for it to follow.

Commit: 7cf5c6f.
<!-- SECTION:FINAL_SUMMARY:END -->
