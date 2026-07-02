---
id: TASK-0185
title: haversine_m returns NaN near antipodes; median() unwrap panics on it
status: To Do
assignee: []
created_date: '2026-07-02 04:50'
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
- [ ] #1 haversine_m returns a finite value in [0, piR] for all finite inputs incl exact and perturbed antipodes (clamp a to [0,1] or use atan2 form); regression test with the antipodal pair
- [ ] #2 median() cannot panic on NaN (use f64::total_cmp)
- [ ] #3 proptest over near-antipodal pairs asserts finiteness
<!-- AC:END -->
