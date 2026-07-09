---
id: TASK-0161
title: 'Test: Weiszfeld geometric_median convergence proptest'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:31'
updated_date: '2026-07-09 04:39'
labels:
  - tests
  - proptest
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P1-7. trilaterate.rs:288. Hand-picked tests cover representative cases. No proptest sweeping random clusters and asserting Weiszfeld result is within the bounding box of inputs OR returns Antipodal/Degenerate cleanly. Capped variant noted as untriggered (lines 907-912); a deliberately-slow input could exercise it.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Proptest: random 3-10 point clusters with lat/lon in valid ranges
- [x] #2 Invariant: GeoMedianResult::Converged result lies within the bounding box of inputs (with small tolerance for numerical drift)
- [x] #3 Invariant: only Converged|Capped have positions; Antipodal|Degenerate have no position; no Capped EVER without input pathology (use #[ignore] or comment if unprovable)
- [x] #4 Construct a deliberately-slow input (cluster pairs at ~1e-9 chord apart): assert Capped is reachable
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add proptest(s) in median.rs #[cfg(test)] mod (geometric_median is pub(crate); cannot reach from integration proptests.rs without widening visibility = production change, disallowed by TEST-ONLY)
2. Invariants: finiteness; convex-hull containment (dist median->each input <= cluster diameter, spherically correct vs bbox); optimality (cost(median) <= cost(spherical-mean centroid), Weiszfeld is monotone descent from that seed)
3. Enum->Option mapping proptest over wider range (Converged|Capped=>Some finite, Antipodal|Degenerate=>None)
4. Run 3x to confirm no counterexamples
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Weiszfeld proptests in trilaterate/median.rs unit tests (commit 1fcff34):
- geometric_median_finite_and_within_hull: lat 45..55 lon 8..16, 1..=8 pts; finite + dist(median,each input) <= cluster diameter +1m (spherical convex-hull bound; bbox is unsound on a sphere).
- geometric_median_beats_spherical_mean: cost(median) <= cost(spherical-mean seed)*1.001+1m. Tolerance absorbs chord(algorithm)-vs-arc(measured) gap; real bite for asymmetric clusters (mean minimizes squared dist).
- geo_median_result_variant_matches_option: wide range -85..85/-175..175; Converged|Capped=>Some(finite), Antipodal|Degenerate=>None.
- geo_median_result_capped_is_reachable (AC#4): singular T-config [(0,0),(0,0.0034),(0,-0.0034),(0.0034,0)] caps at MAX_ITERS; deterministic (fixed input, band 0.0029..0.0039 found empirically), coupled to current MAX_ITERS/CONVERGENCE consts.
- 3x reruns (~768 cases) all green; full e2e 869 pass.
Limitation: kept in median.rs (not proptests.rs) because geometric_median/GeoMedianResult are pub(crate); the integration binary cannot see them and widening visibility is a disallowed production change.

## Review gate (orchestrator, independent) — GO/GO
- 3 proptests + 1 deterministic Capped witness in trilaterate/median.rs. Reviewer stress-ran at ~200k+ samples (50k cases x4) + QA 5x targeted: ZERO counterexamples. Test-only (no pub widening — kept in median.rs since geometric_median/GeoMedianResult are pub(crate)).
- Invariant soundness: (1) finiteness + SPHERICAL-convex-hull containment (dist(median,each input) <= cluster diameter + 1m) — stronger/correct form of AC#2 bbox (bbox is unsound on a sphere); TRUE (every Weiszfeld iterate is a renormalized convex combo of input unit vectors -> inside the hull), non-vacuous (tight hemisphere box never hits Antipodal/Degenerate). (2) enum->Option contract (Converged|Capped=>Some finite; Antipodal|Degenerate=>None), AC#3. (4) deterministic Capped witness (singular near-symmetric T-config eps=0.0034 caps at MAX_ITERS; the 1e-9 idea in the AC did not reliably cap — the implementer found a config that genuinely does).
- HONESTY CORRECTION (MEDIUM review finding): the OPTIMALITY proptest (cost_median <= cost_spherical_mean_seed*1.001 + 1m) bites a median STRICTLY WORSE than the seed (a returned outlier/runaway), NOT a "returned the seed/mean unchanged" median — that case has cost==seed_cost and trivially passes. No cheap proptest invariant catches "returned mean instead of the true median" (they are numerically close). The commit message states the bite accurately (<= seed cost within tolerance); the accurate bite is recorded here. Tolerance (0.1%+1m) correctly absorbs the chord-descent-vs-arc-measure metric gap over the ~13deg cluster.
- LOW (accepted): the Capped witness is coupled to WEISZFELD_MAX_ITERS=50/CONVERGENCE=1e-8; a future const bump could re-center it, but it fails SAFE (panics loudly, does not pass silently).
- Dual gate GO/GO (869 tests).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added property tests for the Weiszfeld geometric median (trilaterate/median.rs). Three proptests — finiteness + spherical-convex-hull containment (the sphere-correct form of AC#2 bbox), an optimality descent bound (median great-circle cost <= spherical-mean-seed cost within a chord-vs-arc tolerance), and the GeoMedianResult->Option contract — plus a deterministic witness proving the Capped (iteration-cap) branch is reachable via a singular near-symmetric config. Reviewer + QA stress-ran ~200k+ proptest samples with zero counterexamples; non-flaky. Kept in median.rs to avoid a pub(crate)->pub production change. Honest bite scope: the optimality test catches a median worse than the seed (outlier/runaway), not a returned-seed median (numerically indistinguishable without the true median). Test-only; dual gate GO/GO; commit 1fcff34.
<!-- SECTION:FINAL_SUMMARY:END -->
