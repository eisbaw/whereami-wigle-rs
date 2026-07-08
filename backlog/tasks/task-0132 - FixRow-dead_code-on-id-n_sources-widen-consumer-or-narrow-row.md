---
id: TASK-0132
title: 'FixRow dead_code on id/n_sources: widen consumer or narrow row'
status: To Do
assignee: []
created_date: '2026-05-20 21:21'
updated_date: '2026-07-08 19:18'
labels:
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/db.rs:76-86. FixRow.id and FixRow.n_sources carry #[allow(dead_code)] since segment_fixes doesn't use them. Same anti-pattern task-0060 (PendingAp removal) just fixed: struct wider than any consumer needs. Either expose n_sources in the history response (so users can filter low-confidence segments) or narrow the row.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Decision in task notes: expose n_sources in HistorySegment (preferred — more useful info) OR narrow FixRow
- [ ] #2 If exposed: HistorySegment gains mean_n_sources field on the wire; segment_fixes averages it
- [ ] #3 If narrowed: FixRow shrinks to (at, lat, lon, accuracy_m); db.rs queries select only those columns
- [ ] #4 #[allow(dead_code)] removed
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Triaged 2026-07-08: two-path decision. PREFERRED = expose (AC#2): add mean_n_sources to the HistorySegment WIRE struct (daemon server + whereami-client, serde default for back-compat) + segment_fixes averages n_sources — a wire-format ADDITION touching both crates + client rendering + tests (medium blast radius, wants a non-fatigued context + full dual review gate). Alternative = narrow FixRow (AC#3, simpler, drops the info). Recommend expose. Not started.
<!-- SECTION:NOTES:END -->
