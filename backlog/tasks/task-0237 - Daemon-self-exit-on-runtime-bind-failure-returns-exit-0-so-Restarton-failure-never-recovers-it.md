---
id: TASK-0237
title: >-
  Daemon self-exit on runtime bind failure returns exit 0, so Restart=on-failure
  never recovers it
status: To Do
assignee: []
created_date: '2026-07-03 08:48'
labels:
  - nix
  - correctness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Discovered during task-0204 (system-service Restart=on-failure). When the server task exits on its own at runtime (e.g. TCP --bind hitting EADDRINUSE, after validate() and unix-listener setup have already passed), main.rs takes the server_exited select arm, logs an error (server task exited unexpectedly), then returns Ok(()) -> process exit 0. systemd Restart=on-failure does NOT retry a clean exit 0, so the daemon stays dead — the single-failure-permanently-dead problem 0204 removed, still open for the opt-in bind case.

This exit 0 is deliberate and test-pinned: task-0195 made the bind-failure path a clean non-panic exit, and tests/shutdown.rs bind_failure_exits_cleanly_without_panic asserts status.success(). Closing the gap is a behavior change that must update that test in lockstep.

Fail-fast fix (MPED): when server_exited is true (self-exit, NOT SIGINT/SIGTERM), main should return non-zero — logging an error then exiting 0 is a silent-failure contradiction. server_exited already distinguishes self-exit from signalled shutdown (server_exited=false stays exit 0). Default system deployment (bind=null, socket-activated unix) is unaffected; only the opt-in TCP bind path bites.

Ref: nix/module.nix task-0204 residual comment, whereamid/src/main.rs server_exited arm, whereamid/tests/shutdown.rs, task-0195.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 server task self-exit (e.g. runtime bind EADDRINUSE) makes whereamid exit non-zero so Restart=on-failure retries it
- [ ] #2 SIGINT/SIGTERM shutdown still exits 0 (server_exited=false path unchanged)
- [ ] #3 tests/shutdown.rs bind-failure test updated to expect the new non-zero exit; still asserts no panic (not exit 101)
<!-- AC:END -->
