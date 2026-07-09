---
id: TASK-0153
title: 'Test: trilaterate proptest exercises full production path (no pre-filter)'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:30'
updated_date: '2026-07-09 06:39'
labels:
  - tests
  - proptest
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P0-6. tests/proptests.rs:38 trilaterate_within_cluster_diameter pre-applies filter_outliers before calling trilaterate. But trilaterate already runs filter_outliers internally (trilaterate.rs:399). The property is correct for the inner math; no proptest feeds RAW outlier-laden input to the public API. A refactor disabling the inner filter_outliers call would still pass.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Drop the manual filter_outliers precall in trilaterate_within_cluster_diameter
- [x] #2 Inject random outlier in addition to the cluster (e.g. one AP placed at lat ± 5° from the cluster centroid)
- [x] #3 Assert the centroid stays within the CLUSTER's diameter (not the diameter including the outlier)
- [x] #4 Test fails if someone removes the inner filter_outliers call (verify by temporarily commenting it out)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Rewrite trilaterate_within_cluster_diameter to feed RAW input to trilaterate()
2. Generate tight cluster (>=3 APs, center +/-0.004 jitter, mutually <2000m) + ONE far outlier (~5deg lat away)
3. Call trilaterate(&raw_aps) directly; assert centroid within cluster diameter (excl outlier) + few-metre tol
4. AC#4 mutation: comment internal filter_outliers, confirm FAIL, revert
5. Gate: nix develop -c just e2e, cargo fmt --check, run proptest 3x
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented (commit f581f0c). Rewrote trilaterate_within_cluster_diameter to feed RAW outlier-laden input to the PUBLIC trilaterate() (no filter_outliers precall).

Generation: tight cluster of 3..8 APs = center (lat -80..80, lon -170..170) + per-AP jitter +/-0.004deg (~<=450m); max intra-cluster pairwise <~1.3km < NEIGHBOR_RADIUS_M(2000m) at all latitudes, so stage-1 drop_isolated keeps the whole cluster and the all-isolated bypass never fires. Plus ONE outlier at center_lat+5deg (~555km) -> isolated -> dropped by internal filter_outliers.

Assertion: for every CLUSTER AP (outlier excluded), haversine(centroid, ap) <= cluster_diameter + 2.0m (float slack only).

AC#4 mutation: replaced internal filter_outliers(aps) with aps.to_vec() at mod.rs:207 -> test FAILED (centroid pulled 138927m ~139km toward outlier, cluster diameter 0.0m). Reverted (mod.rs clean, git diff empty).

Verification: proptest 3x -> all pass, no counterexample. nix develop -c just e2e -> 886 passed / 0 failed (baseline 886), clippy -D warnings clean. cargo fmt --check clean. Invariant held for all generated inputs across 3 runs.

## Review gate (orchestrator, independent) — GO/GO
- QA (agent + orchestrator in-thread, redundant): diff is proptests.rs-ONLY (src intact, filter_outliers present at mod.rs:207 — AC#4 mutation reverted); 886/886 (baseline-neutral, one proptest replaced); clippy 0 -D warnings; fmt clean. FLAKE STRESS: target proptest 0 counterexamples across 8 runs / ~13,568 fresh cases (incl. 3x PROPTEST_CASES=4096); no proptest-regressions cache (genuinely entropy-seeded); no full-suite flake.
- Architecture: rigorous geometric proof the invariant holds for ALL inputs — worst-case intra-cluster pairwise ~1259m < NEIGHBOR_RADIUS_M 2000m (741m headroom) so drop_isolated keeps the cluster + the all-isolated/<=2 bypasses never fire; outlier at +5deg lat (~555km) always isolated -> dropped; no near-pole/antimeridian degeneracy (lat center +/-80, lon +/-170 + 0.004deg jitter). AC#4 bite is structurally load-bearing (cluster_diameter computed over cluster APs only, excluding the outlier; assertion iterates cluster APs; an undropped 555km outlier pulls the centroid ~139km out of the ~1.3km bound). +2m tolerance is float-only (real miss sub-um). Genuine geometric truth (weighted spherical mean in convex hull of survivors subset of cluster hull).
- No findings. filter_outliers import still used by 2 other proptests (no orphan).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Strengthened the trilaterate_within_cluster_diameter proptest to exercise the PUBLIC trilaterate() outlier-rejection path instead of pre-filtering. Previously it called filter_outliers manually before trilaterate (which runs filter_outliers internally at mod.rs:207), so a refactor disabling the inner filter would still pass.

Now: generate a TIGHT cluster (3..8 APs, +/-0.004deg jitter so all are mutually within NEIGHBOR_RADIUS_M=2000m and survive drop_isolated without triggering the all-isolated bypass) PLUS one far outlier (center_lat+5deg ~555km, isolated -> dropped by the internal filter); feed the RAW set to trilaterate(); assert the centroid is within the CLUSTER diameter (max pairwise haversine over cluster APs only) + 2m float slack. If the internal filter fails to drop the outlier, the centroid is pulled ~139km out of bounds and the test fails.

AC#4 verified: commenting out the internal filter_outliers made the test FAIL (~139km miss), then reverted (committed src intact). Reviewer proved the invariant is a genuine geometric truth (weighted spherical mean lies in the convex hull of survivors subset of cluster hull) with no false-failure or flake vector; QA ran ~13,568 fresh proptest cases across 8 runs with zero counterexamples.

Test-only, test-count-neutral. Dual gate GO/GO (886). Commit f581f0c.
<!-- SECTION:FINAL_SUMMARY:END -->
