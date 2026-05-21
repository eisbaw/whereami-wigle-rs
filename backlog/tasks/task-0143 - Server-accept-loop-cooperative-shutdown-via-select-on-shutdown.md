---
id: TASK-0143
title: 'Server accept loop: cooperative shutdown via select! on shutdown'
status: To Do
assignee: []
created_date: '2026-05-20 21:26'
labels:
  - robustness
  - shutdown
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:316-332 + src/main.rs:192-205. main.rs calls server_handle.abort() before drain. The accept loop has no shutdown branch — it relies on abort. Forced cancellation can interrupt a connection mid-write; a tokio::select! with shutdown.notified() would let the loop exit at the next accept boundary cleanly.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Server accept loop wraps the accept in tokio::select! { _ = listener.accept() => {...}, _ = state.shutdown.notified() => break }
- [ ] #2 main.rs removes server_handle.abort(); the shutdown notify propagates cleanly to all loops including server
- [ ] #3 5s drain budget (matches task-0075 for other loops) covers server exit too
- [ ] #4 Test: spawn the server, send SIGTERM (or notify), assert it exits within 5s without abort
<!-- AC:END -->
