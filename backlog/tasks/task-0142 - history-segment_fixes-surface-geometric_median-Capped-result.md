---
id: TASK-0142
title: 'history segment_fixes: surface geometric_median Capped result'
status: To Do
assignee: []
created_date: '2026-05-20 21:25'
labels:
  - observability
  - history
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/history.rs:84-110. Calls geometric_median with .unwrap_or((cur[0].lat, cur[0].lon)) on None. The thin adapter collapses Capped|Converged into Some, Antipodal|Degenerate into None. If Weiszfeld hit the iteration cap with a not-yet-converged iterate, segmentation uses it without warning. The wrong iterate could shift segment-membership (member vs new-segment) silently.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 history::segment_fixes calls geometric_median_detailed (the rich variant from task-0085), not the thin Option adapter
- [ ] #2 Match on Converged: use it. Capped: tracing::debug log including the iterate and seg_id, use it best-effort. Antipodal/Degenerate: tracing::warn, fall back to cur[0]
- [ ] #3 geometric_median_detailed becomes pub(crate) reachable from history.rs (depends on task-0112 lib/bin resolution, or via crate-local path)
- [ ] #4 Test: history with a pathological cluster triggers Capped — verify the debug log fires via custom subscriber
<!-- AC:END -->
