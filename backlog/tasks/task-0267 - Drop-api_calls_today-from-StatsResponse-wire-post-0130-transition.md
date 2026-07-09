---
id: TASK-0267
title: Drop api_calls_today from StatsResponse wire (post-0130 transition)
status: To Do
assignee: []
created_date: '2026-07-09 00:58'
labels:
  - wigle
  - protocol
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0130 renamed the WiGLE daily counter to wigle_lookups_today and, for one transitional release, the daemon emits BOTH api_calls_today and wigle_lookups_today with the same value so old clients keep working. Once all clients have upgraded to read wigle_lookups_today, remove the deprecated api_calls_today field from whereami-protocol StatsResponse, the daemon fill (server.rs), the client render fallback, and the wire fixtures/tests.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 api_calls_today field removed from StatsResponse
- [ ] #2 daemon no longer emits api_calls_today
- [ ] #3 wire fixtures/tests updated to wigle_lookups_today only
<!-- AC:END -->
