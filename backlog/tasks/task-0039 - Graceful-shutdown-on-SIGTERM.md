---
id: TASK-0039
title: Graceful shutdown on SIGTERM
status: To Do
assignee: []
created_date: '2026-05-10 05:38'
labels:
  - daemon
  - reliability
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
main.rs only listens for ctrl_c() (SIGINT). Under systemd a stop sends SIGTERM and the daemon will be SIGKILL'd after the timeout, potentially mid-write to SQLite or mid-API-call. Add a signal handler for SIGTERM (and SIGHUP if useful) that triggers the same graceful shutdown path as SIGINT. Use tokio::signal::unix::signal.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Daemon exits cleanly on SIGTERM as well as SIGINT
- [ ] #2 In-flight DB writes complete or are rolled back; no partial pending rows
- [ ] #3 JoinHandle / CancellationToken approach so spawned resolve tasks finish or are cancelled cleanly
<!-- AC:END -->
