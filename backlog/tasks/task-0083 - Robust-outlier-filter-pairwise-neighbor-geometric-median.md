---
id: TASK-0083
title: 'Robust outlier filter: pairwise neighbor + geometric median'
status: Done
assignee:
  - '@mpedersen'
created_date: '2026-05-13 20:52'
updated_date: '2026-05-13 20:59'
labels:
  - robustness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
filter_outliers() in trilaterate.rs uses an unweighted spherical mean as the cluster center, then thresholds on max(200m, 3 * median_distance). This is non-robust by design: a single catastrophic outlier (e.g. WiGLE returning a Brazilian position for a randomized BSSID seen in Copenhagen) pulls the spherical mean thousands of km away, which inflates the median, which inflates the threshold, which prevents rejecting the very outlier that poisoned the centroid. Real-world incident: locate returned (55.71, 12.57) ±916m at Drejogade when the user was at Strandboulevarden 95 — one cached AP F6:B1:9C:0A:3A:60 was stored at (-12.89, -38.29) Salvador, Brazil. Deleting the cache row immediately recovered ±11m accuracy at the correct location. Fix in two stages: (1) drop APs with no neighbor within NEIGHBOR_RADIUS_M (default 2 km) — catches catastrophic outliers without relying on a poisoned centroid; (2) replace the spherical mean in filter_outliers with a geometric median (Weiszfeld's algorithm) on the unit sphere — ~50% breakdown point vs the arithmetic mean's 0%.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Add drop_isolated() pre-filter step that removes APs with no neighbor within 2 km haversine; if it would drop every AP (sparse rural cluster), fall back to keeping all
- [x] #2 Add geometric_median() over unit-vector representation via Weiszfeld iterations (max 50, convergence threshold 1e-8 chord distance)
- [x] #3 filter_outliers() uses geometric_median for the cluster center instead of the spherical mean
- [x] #4 New unit test reproduces the Brazil-in-Copenhagen incident: 6 Copenhagen APs + 1 Brazil AP, asserts the Brazil AP is dropped and the centroid lands in Copenhagen
- [x] #5 Existing trilaterate tests still pass (antimeridian, polar, antipodal-fallback, no-outliers-all-close, moved-router)
- [x] #6 Existing proptests still pass (trilaterate_within_cluster_diameter)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add drop_isolated() pre-filter in trilaterate.rs (Brazil-catcher)\n2. Add geometric_median() via Weiszfeld in unit-vector space\n3. Wire both into filter_outliers(): drop_isolated -> geometric_median -> threshold\n4. Add Brazil-incident regression test\n5. Run full test suite, clippy, fmt\n6. mped-architect and qa-test-runner review\n7. Commit
<!-- SECTION:PLAN:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Two-stage robust outlier filter to fix the Brazil-in-Copenhagen bug.

Motivation: real-world incident. `whereami locate` returned (55.71, 12.57) ±916m at Drejogade when the user was at Strandboulevarden 95. One cached BSSID (F6:B1:9C:0A:3A:60, a randomized client MAC) had a stale WiGLE position in Salvador, Brazil. The prior single-stage filter (unweighted spherical mean centroid + median-distance threshold) has a 0% breakdown point: one ~10000km outlier pulls the centroid ~1500km from the cluster, inflating the median so the threshold cannot reject the very outlier that poisoned it. Deleting the cache row immediately recovered ±11m accuracy.

Changes in `whereamid/src/trilaterate.rs`:
- Stage 1 `drop_isolated()`: drop APs with no neighbor within NEIGHBOR_RADIUS_M=2km. Centroid-independent so it cannot be poisoned. Falls back to keeping all if every AP is isolated (sparse rural cluster).
- Stage 2 `geometric_median()`: Weiszfeld iterations on unit-vector space (max 50, 1e-8 chord convergence). ~50% breakdown vs spherical mean 0%. Seeds from spherical mean, handles singular spike (1e9 weight cap) and antipodal cancellation.
- Stage 2 threshold: max(STAGE2_FLOOR_M=200m, 3 * median_dist_from_geomedian).
- Named constants: NEIGHBOR_RADIUS_M, STAGE2_FLOOR_M, STAGE2_MEDIAN_MULTIPLIER, WEISZFELD_MAX_ITERS, WEISZFELD_CONVERGENCE.

Tests added:
- `brazil_in_copenhagen_incident`: 6 Copenhagen APs + 1 Brazil AP (strongest signal -49 dBm). Asserts the Brazilian is dropped and centroid lands within ~1km of Strandboulevarden with <100m accuracy.
- `drop_isolated_drops_lone_outlier`: pins stage-1 directly.
- `drop_isolated_falls_back_when_all_isolated`: pins the sparse-cluster bypass.
- `geometric_median_resists_minority_outliers`: 5 clustered + 4 trans-continental outliers; asserts result stays in cluster.

All prior tests still pass (antimeridian, polar, antipodal-fallback, moved-router, no-outliers-all-close, proptest outlier_filter_never_empty).

Reviewed by qa-test-runner and mped-architect in parallel; both said ship. Polish recommendations folded in (named constants, direct helper tests). Remaining recommendations (decoupling helpers from PositionedAp, tracing logs at silent fallbacks) deferred as out of scope.
<!-- SECTION:FINAL_SUMMARY:END -->
