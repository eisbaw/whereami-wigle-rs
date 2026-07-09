---
id: TASK-0268
title: Rename api_counter_corruption_denials wire field to wigle_ vocabulary
status: To Do
assignee: []
created_date: '2026-07-09 01:18'
labels:
  - refactor
  - wigle
  - wire-format
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0130 review LOW: the daily-quota vocabulary moved to wigle_ (wigle_lookups_today), but StatsResponse.api_counter_corruption_denials keeps the generic api_ prefix. It is a distinct concept (in-memory count of WiGLE reservations denied because the persisted same-day counter was malformed, not the daily quota itself), but if the goal is fully WiGLE-specific wire vocabulary it should become e.g. wigle_counter_corruption_denials. Wire-compat: emit both for one release, same as task-0130 did for the counter, then drop via a task like task-0267.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Decision recorded: rename to wigle_-prefixed name OR keep generic with a justifying doc-comment
- [ ] #2 If renamed: StatsResponse emits both old+new for one transitional release (serde default both), follow-up filed to drop the old field
<!-- AC:END -->
