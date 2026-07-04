---
id: TASK-0241
title: Server-task panic exits 0; systemd Restart=on-failure will not recover it
status: To Do
assignee: []
created_date: '2026-07-04 17:43'
labels:
  - daemon
  - robustness
dependencies:
  - TASK-0233
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Found in the task-0233 review. With task-0233 closing the TCP-bind-failure-exits-0 case, the only remaining way the server task "ends on its own" is a PANIC in the accept loop: JoinError -> main logs "server task join error" -> server_exited=true -> main returns Ok(()) -> exit 0. Under systemd Restart=on-failure (task-0204) a clean exit 0 is NOT retried, so a panicked server task leaves a dead daemon reported as success — the SAME hazard task-0233 just closed for the bind case, still open for a panic. Pre-existing (task-0195 architecture), low probability (accept errors are caught, handlers detached), out of task-0233 scope. Fix direction: when the server task terminates unexpectedly (an unexpected Ok(()) OR a JoinError), main should exit NON-ZERO so on-failure recovers it, while keeping the normal SIGTERM/SIGINT path at exit 0. Ref whereamid/src/main.rs server_handle select! arm (~L248-256) and the post-drain return.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 an unexpected server-task termination (unexpected Ok, or a JoinError/panic) causes a non-zero process exit
- [ ] #2 a normal SIGTERM/SIGINT shutdown still exits 0 (the graceful path is unaffected)
- [ ] #3 a test covers the server-task-fails-exits-nonzero path, or documents why it cannot be triggered deterministically
<!-- AC:END -->
