---
id: TASK-0275
title: >-
  Drop api_counter_corruption_denials from StatsResponse wire (post-0268
  transition)
status: To Do
assignee: []
created_date: '2026-07-09 08:01'
labels:
  - refactor
  - wigle
  - wire-format
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Post-0268 cleanup: task-0268 added the WiGLE-specific wigle_counter_corruption_denials wire field and kept api_counter_corruption_denials as a transitional DEPRECATED dual-emit mirror (same value) for one release, mirroring task-0130/task-0267. Once every deployed client reads wigle_counter_corruption_denials, drop the deprecated api_counter_corruption_denials field from StatsResponse, the daemon dual-emit in whereamid/src/server.rs, wire fixtures/tests, and docs/protocol.md. Protocol stays v=1 (field removal is tolerated by serde default on the reader side).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 api_counter_corruption_denials removed from StatsResponse and daemon stop emitting it
- [ ] #2 Wire tests + docs/protocol.md updated to drop the deprecated mirror; only wigle_counter_corruption_denials remains
<!-- AC:END -->
