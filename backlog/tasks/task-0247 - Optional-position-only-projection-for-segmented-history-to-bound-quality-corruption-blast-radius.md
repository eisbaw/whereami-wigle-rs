---
id: TASK-0247
title: >-
  Optional position-only projection for segmented history to bound
  quality-corruption blast radius
status: To Do
assignee: []
created_date: '2026-07-07 23:00'
updated_date: '2026-07-07 23:00'
labels:
  - history
  - robustness
dependencies:
  - TASK-0245
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Segmented history (the default, non-raw view) reads through get_fixes_in_range_guarded, which decodes the v7 quality bundle and hard-errors on any corrupt quality column. So a single detectably-corrupt quality value in range fails the whole position-only segmented view, even though segmentation ignores quality. This is a deliberate fail-loud choice today (production only writes validated bundles via FixQuality::new; DB is single-writer 0600) and is documented at the read site in server.rs handle_history. This task tracks an OPTIONAL narrowing: give segmentation a position-only SELECT projection (id, at, lat, lon, accuracy_m, n_sources) that never touches the quality columns, so quality corruption cannot block position-only history. Weigh against the cost of a second read path diverging from the guarded one. Filed as a forward-carried finding from task-0245's architecture review.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Decide and document whether segmented history should tolerate corrupt quality columns; if yes, add a position-only projection that excludes the five quality columns
- [ ] #2 If implemented: segmented history over a range containing a corrupt-quality row still returns position-only segments; raw history over the same range still fails loudly
- [ ] #3 A regression test pins whichever behavior is chosen
<!-- AC:END -->
