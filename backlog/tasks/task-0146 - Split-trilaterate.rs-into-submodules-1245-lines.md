---
id: TASK-0146
title: Split trilaterate.rs into submodules (1245 lines)
status: To Do
assignee: []
created_date: '2026-05-20 21:28'
labels:
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/trilaterate.rs: 1245 lines, near practical reading-comfort ceiling. Tasks 0055/0056 split db.rs and server.rs; trilaterate.rs is next. Natural split: centroid (weighted spherical mean), outliers (drop_isolated + filter_outliers), median (Weiszfeld + GeoMedianResult).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Split into trilaterate/mod.rs (re-exports) + centroid.rs + outliers.rs + median.rs (or chosen-naming)
- [ ] #2 Tests stay with their modules (each submodule has its own #[cfg(test)] mod)
- [ ] #3 Kani harnesses move accordingly
- [ ] #4 Existing API unchanged: pub use trilaterate::*; from lib.rs/main.rs continues to work
- [ ] #5 Composes with task-0124 (rename to weighted_centroid) — coordinate
<!-- AC:END -->
