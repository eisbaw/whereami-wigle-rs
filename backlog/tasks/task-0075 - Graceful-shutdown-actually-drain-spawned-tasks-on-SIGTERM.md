---
id: TASK-0075
title: 'Graceful shutdown: actually drain spawned tasks on SIGTERM'
status: To Do
assignee: []
created_date: '2026-05-10 11:01'
labels:
  - reliability
  - server
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0039 added a SIGTERM handler but the implementation just exits main; spawned tokio tasks (Nominatim backfill, resolve_background, pending drain) are cut off mid-flight. SQLite is transactional so corruption is unlikely, but the task summary said 'graceful' and the implementation is 'exit fast'. Add a tokio::sync::CancellationToken that all spawned loops await; on SIGTERM, cancel + JoinSet::join_all with a timeout. Found in v0.4.0 review (keeper).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 SIGTERM cancels via CancellationToken; spawned tasks observe and exit cleanly
- [ ] #2 Daemon waits up to N seconds (configurable, default 5) for tasks to drain before exiting
- [ ] #3 Test verifies in-flight DB writes complete before process exit
<!-- AC:END -->
