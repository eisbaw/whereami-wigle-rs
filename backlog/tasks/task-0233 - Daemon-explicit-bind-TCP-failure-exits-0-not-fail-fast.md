---
id: TASK-0233
title: 'Daemon: explicit --bind TCP failure exits 0, not fail-fast'
status: To Do
assignee: []
created_date: '2026-07-03 05:49'
updated_date: '2026-07-03 09:56'
labels:
  - daemon
  - robustness
dependencies: []
priority: medium
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Review gate (cycle 8, mped-architect M1): rated MEDIUM, acceptable-as-filed but prioritize BEFORE anyone deploys with --bind. Preferred fix direction from review: graceful degrade (keep serving the mandatory unix socket, warn loudly on TCP failure) rather than making TCP failure fatal — the optional transport must never kill the mandatory one. Note the hm unit has Restart=on-failure (exit 0 = no restart) and the system unit has no Restart= at all (task-0204), so today the failure mode is a dead daemon reported as success.

Review gate (cycle 10, mped M-1): task-0237 (filed during 0204) turned out to be a DUPLICATE of this task with a CONTRADICTORY fix direction (make the self-exit non-zero). 0237 is archived; this task is canonical. Reaffirmed direction: graceful degrade — keep serving the mandatory unix socket, warn loudly when the opt-in TCP bind fails; do NOT make it fatal/non-zero (a busy opt-in TCP port would crash-loop the WHOLE daemon to start-limit-hit under the new task-0204 Restart=on-failure + startLimitBurst=5/60s). Context absorbed from 0237: the residual is now also documented at the nix/module.nix Restart comment (updated in cycle-10 hardening to state the graceful-degrade direction); with Restart=on-failure the exit-0 self-exit is exactly the case systemd will NOT retry, which is why this task matters more post-0204. Sites unchanged: server.rs run_server TCP bind, main.rs server_exited arm, tests/shutdown.rs bind_failure_exits_cleanly_without_panic (must be updated to pin the degrade behavior).

ID-RECYCLE NOTE: after the duplicate 0237 was archived, the backlog CLI reused id TASK-0237 for an UNRELATED new task (sigterm test de-flake, QA cycle-10 H1). References to the duplicate above mean the ARCHIVED 0237, not the live one.
<!-- SECTION:NOTES:END -->
