---
id: TASK-0032
title: Fix antimeridian/polar trilateration bug
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 05:37'
updated_date: '2026-05-10 06:53'
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
- [x] #1 Two APs at (0, 179) and (0, -179) trilaterate to lon ≈ ±180, not 0
- [x] #2 Two APs at (89, 0) and (89, 180) trilaterate to a polar point, not lat=89 lon=90
- [x] #3 Existing trilateration tests still pass
- [x] #4 Property test covers antimeridian and polar generators
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add to_unit_vec / from_unit_vec helpers
2. Replace weighted lat/lon mean in trilaterate() with weighted 3D unit-vector mean + normalize back to lat/lon
3. Replace lat/lon median in filter_outliers() with spherical mean center
4. Guard against degenerate (near-zero magnitude) sums (antipodal APs)
5. Add tests: antimeridian (lon ±179), polar (lat 89), bounded existing tests still pass
6. Update proptest trilaterate_within_bounds to bounded-range hypothesis or relax to spherical convex hull
7. cargo test, clippy, fmt, fuzz_trilaterate 30s
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented spherical mean (3D unit-vector centroid) for both filter_outliers and trilaterate. Added antipodal-cancellation guard in trilaterate that falls back to the strongest-signal AP and inflates accuracy to 1000m when the weighted vector sum has near-zero magnitude. Added 4 new unit tests (antimeridian, antimeridian-outliers, polar, antipodal-fallback). Updated proptest trilaterate_within_bounds → trilaterate_within_cluster_diameter to use a spherically-correct property (centroid haversine distance to every AP <= cluster diameter), since the lat/lon-bounding-box property is mathematically wrong on a sphere — great-circle arcs can extend to higher latitudes than either endpoint.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced naive arithmetic-mean centroid in filter_outliers and trilaterate with the spherical mean (project to unit 3-vector, weighted-sum, normalize, project back). Eliminates the antimeridian bug (two APs at lon=±179 used to centroid to lon=0) and makes polar inputs sensible.

Changes:
- trilaterate.rs: to_unit_vec/from_unit_vec helpers, spherical-mean centroid in trilaterate(), spherical-mean center in filter_outliers(), antipodal-cancellation guard with strongest-signal fallback (accuracy_m=1000m to flag ambiguity).
- tests: 4 new unit tests covering antimeridian, antimeridian-outliers, polar, antipodal-fallback. All existing tests pass unchanged.
- proptests.rs: replaced trilaterate_within_bounds (lat/lon bounding box, mathematically wrong on a sphere) with trilaterate_within_cluster_diameter (haversine distance to every input <= cluster diameter).

Tests: cargo test pass (42 unit + 9 proptest). cargo clippy --all-targets -- -D warnings clean. cargo fmt clean. fuzz_trilaterate 30s = 109,930 runs, no crashes.
<!-- SECTION:FINAL_SUMMARY:END -->
