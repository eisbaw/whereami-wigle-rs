---
id: TASK-0087
title: 'Amortize geometric median in history::segment_fixes'
status: Done
assignee: []
created_date: '2026-05-13 21:54'
updated_date: '2026-07-05 02:15'
labels:
  - performance
  - history
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0084 swapped the arithmetic-mean stay-point centroid for geometric_median in history.rs::segment_fixes, but the call site runs Weiszfeld (up to 50 iters) once PER fix in the pass. For each new fix considered: marshal cur to coords, run full geometric_median, use the result for one distance check, then if the fix joins the segment, repeat from scratch with one more point. Worst case O(N^2 * max_iters * |cur|) per pass; with ~10k fixes over 7 days the geometric median is recomputed thousands of times even though it is largely stable inside a long stay-point. Hot path: handle_history serves the running estimate to the user. Both QA and MPED reviews of task-0084 missed this — the call site looks syntactically tiny ('just call geometric_median instead of arithmetic_mean') but it upgrades an O(1) running stat to O(50·N) repeated work. Fix options: (a) only recompute when the running estimate drifts by some threshold from the last computed centroid; (b) update an incremental running estimate (single Weiszfeld iteration per new point); (c) accept stay-point membership against a cheaper running estimate (arithmetic spherical mean) and only run the geometric median once at flush() time for the segment's reported centroid. Option (c) is the cleanest separation: outlier robustness only matters for the *reported* centroid; membership decisions can use the cheap mean.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 segment_fixes runs geometric_median at most O(N) times across a full pass (not O(N^2))
- [x] #2 Existing history tests still pass; if any depended on exact arithmetic-mean intermediate centroids, update to the new behavior
- [x] #3 Add a benchmark or proptest that documents the asymptotic cost (e.g. 1000-fix segmentation completes in <100 ms on the developer laptop)
- [x] #4 Reported segment centroid remains the geometric median (outlier-robust); only membership decisions may use the cheaper estimate
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Replace per-fix geometric-median membership recomputation with an O(1) running spherical-vector mean while keeping one geometric median per flushed reported segment.
2. Add cancellable internal iteration seams needed by TASK-0201 without changing the existing public behavior.
3. Add deterministic complexity accounting for 1,000 fixes, geometric-median outlier-output coverage, and antimeridian membership coverage.
4. Run full Nix end-to-end and mandatory parallel QA/architecture gates before a dedicated TASK-0087 commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
2026-07-05 dependency review for TASK-0201: QA and architecture independently found that spawn_blocking alone would relocate rather than remove the O(n^2) CPU amplification. TASK-0087 is therefore being completed first as a separate dependency commit. The accepted design uses an incremental spherical mean only for membership and retains the geometric median for every reported segment centroid.

2026-07-05 verification: the 1,000-fix structural regression now records exactly one median call and 1,000 total coordinate inputs instead of the old 500,500; a 64-case proptest asserts total median inputs and calls are bounded by N for generated partitions. Focused history, trilateration, cancellation stress, parallel stress, release tests, and the full 402-test Nix e2e suite passed. Mandatory QA and architecture gates both returned GO after stale source semantics and forward tracker premises were corrected.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced per-fix geometric-median recomputation with an O(1) running spherical-vector mean for membership while retaining one robust geometric median per reported segment at flush. Added explicit cancellation through segmentation and every Weiszfeld traversal, deterministic linear-work accounting, proptest coverage, and regressions for outliers, antimeridian, poles, degenerate sums, dropped segments, and non-finite inputs. Prepared the cancellation seam for TASK-0201 and recorded narrowed follow-up scope in TASK-0201 and duplicate observability tasks TASK-0141/0142.
<!-- SECTION:FINAL_SUMMARY:END -->
