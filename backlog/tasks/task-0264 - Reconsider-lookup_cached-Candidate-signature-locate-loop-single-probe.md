---
id: TASK-0264
title: 'Reconsider lookup_cached &[Candidate] signature + locate loop single-probe'
status: To Do
assignee: []
created_date: '2026-07-08 21:52'
labels:
  - refactor
  - server
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0140 (commit 01f748a). Two deferred items:
1. AC #2 of task-0140 suggested lookup_cached take &[Candidate] and return positioned rows directly. Deferred: it would ripple into resolver.rs autolocate caller (~line 1228) and the lookup_cached_readonly sibling, both &[String], for no readability gain. Revisit only if a future change already touches those callers.
2. To hold handle_locate net line delta at 0, the positioned_aps loop reads the signal via active.get(bssid).map(...) inside a branch already gated by active.contains_key(bssid) — a second (cheap) probe of a small in-memory map. A single-lookup `if let Some(candidate) = active.get(...)` bind is marginally cleaner but costs +1 line. Decide whether the +1 line is acceptable for the single-lookup form.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Decide lookup_cached signature (keep &[String] or move to &[Candidate]) with rationale recorded
- [ ] #2 Decide locate loop form (double-probe vs single if-let bind) and apply chosen form
<!-- AC:END -->
