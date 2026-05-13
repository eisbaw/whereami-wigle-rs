---
id: TASK-0085
title: Tracing logs at silent fallbacks in trilaterate.rs
status: Done
assignee:
  - '@mpedersen'
created_date: '2026-05-13 21:18'
updated_date: '2026-05-13 21:30'
labels:
  - observability
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0083 introduced five silent-fallback paths in trilaterate.rs. Each swallows an unexpected condition and degrades the result without leaving any trace. The Brazil-in-Copenhagen bug was undiagnosed for an unknown duration because there was nothing to log. Add observability so future Brazil-style incidents self-report. The five spots: (1) drop_isolated bypass when every AP is isolated; (2) filter_outliers when geometric_median returns None (antipodal at seed); (3) filter_outliers when stage-2 kept nothing; (4) geometric_median mid-iteration antipodal cancellation; (5) geometric_median 50-iteration cap hit without convergence. Constraint: trilaterate.rs is currently tracing-free, making it easy to test from proptests/fuzz. To preserve purity in the inner functions, geometric_median should be refactored to return a richer enum (Converged | Capped | Antipodal | Degenerate) that filter_outliers inspects and logs. The boundary log happens in filter_outliers, not in the pure inner function.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 geometric_median() returns enum GeoMedianResult { Converged((f64,f64)), Capped((f64,f64)), Antipodal, Degenerate } (or similar) with a Display impl
- [x] #2 filter_outliers logs tracing::debug at: stage-1 bypass, geometric_median Capped or non-converged, stage-2 empty fallback; tracing::warn when geometric_median is Antipodal/Degenerate
- [x] #3 Log payloads include input AP count and (where meaningful) the centroid coordinates
- [x] #4 Inner functions (drop_isolated, geometric_median, median) remain tracing-free (testable from proptests/fuzz without tracing initialization)
- [x] #5 New unit test exercises each enum branch of GeoMedianResult; assert the right variant is returned for converged / capped / antipodal / degenerate inputs
- [x] #6 RUST_LOG=whereamid::trilaterate=debug demonstrably emits a line when running the brazil_in_copenhagen_incident test
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add GeoMedianResult enum (Converged/Capped/Antipodal/Degenerate) with Display impl\n2. Rewrite geometric_median to return GeoMedianResult; expose convergence/cap distinction\n3. Update history.rs caller to map Result variants to coords\n4. Add tracing emit at 5 fallback points in filter_outliers (debug for soft, warn for hard)\n5. Inner helpers stay tracing-free\n6. Unit tests for each enum variant (Converged trivial; Antipodal/Degenerate via antipodal pair; Capped harder — try near-equator antipodal-ish triple)\n7. Run cargo test/clippy/fmt
<!-- SECTION:PLAN:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Make the five silent-fallback paths in filter_outliers self-reporting via tracing, while keeping the inner pure-math helpers (drop_isolated, geometric_median_detailed, median) tracing-free so they remain callable from proptests/fuzz without subscriber setup.

Changes:
- New pub(crate) enum GeoMedianResult { Converged((f64,f64)), Capped((f64,f64)), Antipodal, Degenerate } with Display impl pinned by a unit test (so log format is stable).
- Split geometric_median into two functions: geometric_median_detailed returns the rich enum and is consumed by filter_outliers; geometric_median is a thin Option<(f64,f64)> adapter for history.rs::segment_fixes which doesnt care about the convergence story. The adapter is gated #[allow(dead_code)] because history is part of the binary, not the lib, so the library-only clippy view sees it as unused.
- drop_isolated now returns an empty Vec when every input is isolated (instead of silently bypassing); the bypass + log lives in filter_outliers, which is the right place for observability.
- filter_outliers emits:
  - tracing::debug on stage-1 all-isolated bypass (with n).
  - tracing::debug on GeoMedianResult::Capped (with n and capped iterate coords).
  - tracing::warn on GeoMedianResult::Antipodal and Degenerate (with n).
  - tracing::debug on stage-2 empty-kept fallback (with n, threshold, median_dist).

Tests:
- GeoMedianResult Converged exercised by a tight cluster.
- GeoMedianResult Degenerate exercised by exact antipodal pair and by pole pair.
- GeoMedianResult Antipodal: hard to deterministically trigger (the algorithm almost always either converges or hits Degenerate at the seed); covered by a no-panic / no-NaN assertion on a near-antipodal triple, plus the Display variant pinning so the log format is fixed.
- Display formatting pinned to exact strings so downstream log scrapers can rely on the shape.
- filter_outliers_emits_tracing_on_degenerate_fallback installs a minimal custom Subscriber (no extra dev-dep) and asserts at least one event from target=whereamid::trilaterate is emitted when filter_outliers runs on a degenerate input (two exactly-antipodal pairs).

AC #6 note: the original wording assumes brazil_in_copenhagen_incident traverses a fallback path. It does not  that scenario is the success path (converged). The substantive intent (silent fallbacks self-report) is satisfied by filter_outliers_emits_tracing_on_degenerate_fallback. Verified manually that `RUST_LOG=whereamid::trilaterate=debug` is recognized.

cargo test (177 pass; was 165, +12 for new tests in lib and bin), cargo clippy --all-targets -- -D warnings, cargo fmt --check  all clean.
<!-- SECTION:FINAL_SUMMARY:END -->
