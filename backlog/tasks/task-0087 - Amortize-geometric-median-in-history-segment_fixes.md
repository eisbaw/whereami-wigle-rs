---
id: TASK-0087
title: 'Amortize geometric median in history::segment_fixes'
status: To Do
assignee: []
created_date: '2026-05-13 21:54'
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
- [ ] #1 segment_fixes runs geometric_median at most O(N) times across a full pass (not O(N^2))
- [ ] #2 Existing history tests still pass; if any depended on exact arithmetic-mean intermediate centroids, update to the new behavior
- [ ] #3 Add a benchmark or proptest that documents the asymptotic cost (e.g. 1000-fix segmentation completes in <100 ms on the developer laptop)
- [ ] #4 Reported segment centroid remains the geometric median (outlier-robust); only membership decisions may use the cheaper estimate
<!-- AC:END -->
