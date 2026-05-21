---
id: TASK-0102
title: Track all spawned tokio tasks across shutdown drain
status: To Do
assignee: []
created_date: '2026-05-20 21:11'
labels:
  - robustness
  - shutdown
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/main.rs:236-240 + src/server.rs:494-499 + src/server.rs:511-561. Cooperative shutdown (task-0075) drains the four named loops (scan/drain/prune/server) but every scan-loop iteration spawns a fresh resolve_background task, and handle_locate spawns address-resolution + a fallback resolve_background. These spawns are detached. On SIGTERM the daemon waits 5s for the four named loops; orphaned spawns mid-API-call are silently killed when the process exits, leaving partial writes (e.g. WiGLE quota reserved but no aps row).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Detached spawns moved behind a tokio::task::JoinSet (or similar handle-tracking structure) owned by DaemonState
- [ ] #2 Shutdown drain waits up to 5s for the JoinSet to drain before forced-exit (matches the existing budget from task-0075)
- [ ] #3 Stats counter orphan_tasks_aborted increments on each forced-cancellation at shutdown
- [ ] #4 Test: spawn 5 fake resolve_background tasks that sleep 10s, trigger shutdown after 1s, assert orphan_tasks_aborted == 5 OR the drain waited (whichever the implementation chose)
<!-- AC:END -->
