---
id: TASK-0074
title: Add inflight count to stats response
status: To Do
assignee: []
created_date: '2026-05-10 11:00'
labels:
  - observability
  - server
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
DaemonState.inflight HashSet has no operator visibility. If a provider hangs despite timeouts, the set grows silently. stats handler (server.rs:870-888) already exposes cached/pending/not_found counts; add inflight_count for parity. Trivial change, big debugging payoff later. Found in v0.4.0 review (keeper).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 StatsResponse has inflight_count: usize
- [ ] #2 whereami-client lib + CLI render the new field
<!-- AC:END -->
