---
id: TASK-0233
title: 'Daemon: explicit --bind TCP failure exits 0, not fail-fast'
status: To Do
assignee: []
created_date: '2026-07-03 05:49'
labels:
  - daemon
  - robustness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0196 kept the pre-existing behavior that a TCP --bind failure inside run_server (e.g. EADDRINUSE) is swallowed: run_server returns Err, the spawned server task catches it and returns, and main() treats that as the task-0195 self-exit path -> clean shutdown, exit 0. The unix socket bind is fatal (nonzero) at startup, but an EXPLICIT, user-requested --bind that cannot bind exits 0, so systemd (Restart=on-failure) will NOT restart and the operator sees success. Reason it was left: preserving the task-0195 server-task self-exit architecture and the existing shutdown.rs bind-collision test (which asserts exit 0). Consider: when --bind is explicitly configured and its TCP bind fails, exit NONZERO (fail-fast), while keeping the unix socket the mandatory transport. Also consider whether the daemon should keep serving on the unix socket when only the opt-in TCP bind fails (graceful degrade) rather than shutting down entirely. Sites: whereamid/src/server.rs run_server (TCP bind), whereamid/src/main.rs server_handle arm, whereamid/tests/shutdown.rs bind_failure_exits_cleanly_without_panic.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 an explicitly-configured --bind that fails to bind produces a non-zero exit (or a documented graceful unix-only degrade), not a silent exit 0
- [ ] #2 the unix socket remains the mandatory transport; its bind failure stays fatal
- [ ] #3 shutdown.rs test updated to encode the chosen behavior
<!-- AC:END -->
