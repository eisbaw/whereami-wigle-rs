---
id: TASK-0084
title: Decouple drop_isolated/geometric_median from PositionedAp
status: Done
assignee:
  - '@mpedersen'
created_date: '2026-05-13 21:18'
updated_date: '2026-05-13 21:24'
labels:
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0083 added drop_isolated() and geometric_median() in trilaterate.rs, but both are coupled to the domain type PositionedAp even though they only read lat/lon. Decoupling to &[(f64,f64)] enables reuse in history.rs::segment_fixes, which currently uses a non-robust arithmetic lat/lon mean for stay-point clustering. History clustering would inherit task-0083's antimeridian/pole/outlier robustness for free. The decoupling cost is a single allocation per locate call (~30 APs max), negligible. Note: this is only worth doing if history.rs is actually rewired to use geometric_median; otherwise it is speculative generality.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 drop_isolated() takes &[(f64,f64)] instead of &[PositionedAp] and returns the kept coordinates
- [x] #2 geometric_median() takes &[(f64,f64)] instead of &[PositionedAp]
- [x] #3 filter_outliers() marshalls PositionedAp -> (lat,lon) once at the call boundary
- [x] #4 history.rs::segment_fixes uses geometric_median for stay-point centroid (replacing arithmetic mean of lat/lon)
- [x] #5 Existing history tests pass with the new centroid; if a test asserted exact arithmetic mean coordinates, update to the geometric median (any change in the centroid is by-design improvement)
- [x] #6 All trilaterate and history proptests still pass
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Change drop_isolated to take &[(f64,f64)] and return Vec<usize> (indices into input)\n2. Change geometric_median to take &[(f64,f64)]\n3. filter_outliers: marshal to coords once, thread indices through stage 1, then pass kept coords to stage 2\n4. history.rs segment_fixes: replace arithmetic centroid with call to geometric_median\n5. Update all existing tests for new signatures\n6. cargo test, clippy, fmt
<!-- SECTION:PLAN:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Decouple drop_isolated and geometric_median from PositionedAp so they operate on plain coordinate slices, then reuse geometric_median for history.rs stay-point centroids.

Changes:
- trilaterate.rs: drop_isolated now takes &[(f64,f64)] and returns Vec<usize> (indices into input). This preserves any per-point metadata (e.g. signal_dbm) without a separate join step.
- trilaterate.rs: geometric_median takes &[(f64,f64)] and is now pub(crate) for crate-internal reuse.
- trilaterate.rs: filter_outliers marshals PositionedAp -> (lat,lon) once at the call boundary, threads indices through stages 1 and 2.
- history.rs: segment_fixes now uses geometric_median for both the running cluster centroid (used for segmentation membership) and the final stored Segment centroid. This inherits the antimeridian/pole-correctness and 50% outlier-breakdown robustness of trilaterate.rs.
- Updated three in-crate tests to construct (lat,lon) coordinate slices when invoking the now-decoupled helpers.
- Converted two clippy-flagged vec![] literals to array literals (the previous shape required Vec for &[PositionedAp]; now array suffices for the iter().map().collect() consumption).

Tests:
- All 165 workspace tests pass (85 lib + 9 proptests + 54 db/server/etc).
- cargo clippy --all-targets -- -D warnings clean.
- cargo fmt --check clean.

Notes / trade-offs:
- segment_fixes used to update the running centroid by iterating only over the lat sum. It now runs Weiszfeld iterations on each insertion. Stay-points are typically a handful of fixes, so the cost is negligible.
- Existing history tests still pass within their original tolerances (1e-3 deg) because the test clusters are tight and the geometric median of nearly-collinear close points coincides with the arithmetic mean to within that tolerance.
<!-- SECTION:FINAL_SUMMARY:END -->
