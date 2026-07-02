---
id: TASK-0185
title: haversine_m returns NaN near antipodes; median() unwrap panics on it
status: Done
assignee:
  - '@claude'
created_date: '2026-07-02 04:50'
updated_date: '2026-07-02 06:10'
labels:
  - correctness
  - math
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
haversine_m computes c = 2*a.sqrt().asin() where a can exceed 1.0 by a few ulps for near-antipodal points (verified: a=1.0000000000000004 -> asin(>1)=NaN; ~5% of pairs within +-1e-7 deg of the antipode). A NaN distance reaches median() at trilaterate.rs:378 which does sorted.sort_by(|a,b| a.partial_cmp(b).unwrap()) -> panic in the locate hot path. Same unwrap fires if any cached AP row holds NaN lat/lon. NaN also corrupts drop_isolated and stay-point membership silently. Ref whereamid/src/geo.rs:15-17, trilaterate.rs:205-216,261,378, history.rs:104-105.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 haversine_m returns a finite value in [0, piR] for all finite inputs incl exact and perturbed antipodes (clamp a to [0,1] or use atan2 form); regression test with the antipodal pair
- [x] #2 median() cannot panic on NaN (use f64::total_cmp)
- [x] #3 proptest over near-antipodal pairs asserts finiteness
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. geo.rs haversine_m: clamp the haversine a term to [0,1] before sqrt/asin. For any finite input this guarantees c in [0, pi] and R*c in [0, piR]; kills the near-antipodal a>1 -> asin(NaN) case.
2. trilaterate.rs median(): replace partial_cmp(...).unwrap() with f64::total_cmp so a NaN element (from an antipodal haversine or a NaN cached AP lat/lon) can never panic the sort.
3. Tests: geo.rs regression with the verified antipodal pair (assert finite + in [0,piR]) and an exact-antipode pair; trilaterate.rs median-with-NaN (deterministic, asserts no panic + finite result).
4. proptest (AC3) in proptests.rs: near-antipodal pair generator asserts haversine_m finiteness.
5. Gate: nix develop -c just e2e + fmt. Commit, no push, no AI attribution.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented (commit 083452f).

- geo.rs haversine_m: a.clamp(0.0, 1.0) before sqrt/asin. For any finite input a in [0,1] => c in [0,pi] => R*c in [0,piR]. NaN inputs still yield NaN (acceptable: AC1 constrains only finite inputs), and median() below is now robust to that anyway.
- trilaterate.rs median(): a.total_cmp(b) replaces partial_cmp().unwrap(). total_cmp is a total order over ALL f64 (NaN sorts to an end), so the sort is panic-free even with NaN from an antipodal haversine or a NaN cached AP coord.

Tests: geo::haversine_finite_near_antipode (the verified counterexample pair; finite + in [0,piR]) and haversine_finite_exact_antipode (a==1.0 exactly, ~piR); trilaterate::median_does_not_panic_on_nan (deterministic: total_cmp puts +NaN last so [1,3,5,NaN] -> even median (3+5)/2=4.0); proptests::haversine_finite_near_antipodal (jittered near-antipode generator).

Gotcha: rustfmt wraps assert!(cond, "..msg..") when the message (esp. with unicode piR / interpolation) crosses 100 cols; had to run cargo fmt. Gate: e2e exit 0, fmt exit 0. Test totals 208 -> 215 (+2 geo +1 median, each double-counted in lib+bin => +6; +1 proptest = +7).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Eliminated a reachable NaN-driven panic in the locate hot path.

Root cause: haversine_m computed c = 2*asin(sqrt(a)); for near-antipodal points a exceeded 1.0 by a few ulps (verified 1.0000000000000004) so asin returned NaN. That NaN reached median() in the stage-2 outlier filter, whose partial_cmp().unwrap() sort then panicked. The same unwrap fired on NaN lat/lon from a cached AP row, and NaN silently corrupted segmentation/drop_isolated.

Changes:
- geo.rs: clamp a to [0,1] before sqrt/asin => haversine_m finite in [0, piR] for all finite inputs.
- trilaterate.rs: median() sorts with f64::total_cmp (total order) instead of partial_cmp().unwrap() => never panics on NaN.

Tests: verified antipodal pair + exact antipode (geo), median-with-NaN (trilaterate), and a near-antipodal finiteness proptest.

User impact: locate no longer risks crashing when APs are near-antipodal or a cached coordinate is NaN.

Gate: just e2e + fmt exit 0; 208 -> 215 tests. Risk: none identified; the clamp only affects the pathological >1 ulp case, and total_cmp matches partial_cmp on all non-NaN data so existing behavior is unchanged.
<!-- SECTION:FINAL_SUMMARY:END -->
