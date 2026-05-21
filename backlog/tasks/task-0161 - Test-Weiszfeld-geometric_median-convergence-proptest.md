---
id: TASK-0161
title: 'Test: Weiszfeld geometric_median convergence proptest'
status: To Do
assignee: []
created_date: '2026-05-20 21:31'
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
- [ ] #1 Proptest: random 3-10 point clusters with lat/lon in valid ranges
- [ ] #2 Invariant: GeoMedianResult::Converged result lies within the bounding box of inputs (with small tolerance for numerical drift)
- [ ] #3 Invariant: only Converged|Capped have positions; Antipodal|Degenerate have no position; no Capped EVER without input pathology (use #[ignore] or comment if unprovable)
- [ ] #4 Construct a deliberately-slow input (cluster pairs at ~1e-9 chord apart): assert Capped is reachable
<!-- AC:END -->
