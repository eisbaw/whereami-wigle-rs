---
id: TASK-0032
title: Fix antimeridian/polar trilateration bug
status: To Do
assignee: []
created_date: '2026-05-10 05:37'
labels:
  - bug
  - trilaterate
  - math
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Weighted-centroid trilateration in trilaterate.rs uses naive arithmetic mean of lat/lon. Two APs at lon=+179 and lon=-179 produce centroid lon=0 (Africa) instead of ±180. Same issue near the poles. Project to 3D unit vectors, weighted-average, project back to lat/lon. Discovered in code review (paper-correctness-style).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Two APs at (0, 179) and (0, -179) trilaterate to lon ≈ ±180, not 0
- [ ] #2 Two APs at (89, 0) and (89, 180) trilaterate to a polar point, not lat=89 lon=90
- [ ] #3 Existing trilateration tests still pass
- [ ] #4 Property test covers antimeridian and polar generators
<!-- AC:END -->
