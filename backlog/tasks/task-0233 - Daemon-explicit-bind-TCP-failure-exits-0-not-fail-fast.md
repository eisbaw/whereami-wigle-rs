---
id: TASK-0233
title: 'Daemon: explicit --bind TCP failure exits 0, not fail-fast'
status: Done
assignee:
  - '@mark'
created_date: '2026-07-03 05:49'
updated_date: '2026-07-04 17:44'
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
- [x] #1 an explicitly-configured --bind that fails to bind produces a non-zero exit (or a documented graceful unix-only degrade), not a silent exit 0
- [x] #2 the unix socket remains the mandatory transport; its bind failure stays fatal
- [x] #3 shutdown.rs test updated to encode the chosen behavior
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. server.rs run_server: make the opt-in TCP bind failure NON-fatal — catch TcpListener::bind Err, warn! loudly, degrade to tcp=None (keep serving the mandatory unix socket) instead of ?-propagating. Keep parsed_bind() parse errors and the unix bind fatal (AC#2).
2. Update the run_server doc comment (~591-595) which currently describes the OLD "TCP failure flows through the task-0195 self-exit path" behavior — it would lie after the change.
3. Rewrite shutdown.rs bind_failure_exits_cleanly_without_panic -> tcp_bind_failure_degrades_to_unix_only: hold the TCP port, spawn daemon, wait_until_socket_ready (unix binds fine), assert the daemon STAYS UP (does not exit), assert stderr has the degrade warning, then SIGTERM -> clean exit (AC#3).
4. Verify nix/module.nix Restart comment still consistent (task notes say cycle-10 already pointed it at graceful-degrade).
5. Gate: nix develop -c just e2e + fmt; run shutdown tests. Commit (no AI attribution) + tracker separate.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Review gate (cycle 8, mped-architect M1): rated MEDIUM, acceptable-as-filed but prioritize BEFORE anyone deploys with --bind. Preferred fix direction from review: graceful degrade (keep serving the mandatory unix socket, warn loudly on TCP failure) rather than making TCP failure fatal — the optional transport must never kill the mandatory one. Note the hm unit has Restart=on-failure (exit 0 = no restart) and the system unit has no Restart= at all (task-0204), so today the failure mode is a dead daemon reported as success.

Review gate (cycle 10, mped M-1): task-0237 (filed during 0204) turned out to be a DUPLICATE of this task with a CONTRADICTORY fix direction (make the self-exit non-zero). 0237 is archived; this task is canonical. Reaffirmed direction: graceful degrade — keep serving the mandatory unix socket, warn loudly when the opt-in TCP bind fails; do NOT make it fatal/non-zero (a busy opt-in TCP port would crash-loop the WHOLE daemon to start-limit-hit under the new task-0204 Restart=on-failure + startLimitBurst=5/60s). Context absorbed from 0237: the residual is now also documented at the nix/module.nix Restart comment (updated in cycle-10 hardening to state the graceful-degrade direction); with Restart=on-failure the exit-0 self-exit is exactly the case systemd will NOT retry, which is why this task matters more post-0204. Sites unchanged: server.rs run_server TCP bind, main.rs server_exited arm, tests/shutdown.rs bind_failure_exits_cleanly_without_panic (must be updated to pin the degrade behavior).

ID-RECYCLE NOTE: after the duplicate 0237 was archived, the backlog CLI reused id TASK-0237 for an UNRELATED new task (sigterm test de-flake, QA cycle-10 H1). References to the duplicate above mean the ARCHIVED 0237, not the live one.

Implemented in commit 815d1b8 (chosen direction: graceful degrade, per the pinned review).
- run_server (server.rs): TcpListener::bind Err is now caught -> loud warn! -> degrade to tcp=None (keep serving the mandatory unix socket) instead of ?-propagating. A malformed --bind still fail-fasts at validate() (config error, non-zero); the unix socket bind stays fatal (setup_unix_listener, before the server task spawns). AC#1+AC#2.
- Test (AC#3): bind_failure_exits_cleanly_without_panic -> tcp_bind_failure_degrades_to_unix_only. Holds the TCP port, waits for "control channel: unix socket ready" (logged AFTER the TCP attempt, so it proves degrade-not-exit), asserts the loud degrade warning is present, asserts the daemon is STILL RUNNING, then SIGTERM -> clean exit 0. mped confirmed it FAILS against the old code (readiness line never appears -> fails fast).
GOTCHA (important, cost me a debug cycle): the daemon logs to STDOUT (tracing_subscriber::fmt default writer), NOT stderr. shutdown.rs previously discarded stdout (Stdio::null) and only kept stderr (which gets Rust panics + a main-returned Err). A stdout-log-line readiness check saw an empty stderr. Fix: spawn_daemon now captures BOTH streams; new read_output()/wait_until_output_contains() read stdout++stderr, robust to a future stdout->stderr switch. Filed task-0240 to decide/document the stdout logging.
Review gate (measured): qa GO (319 tests, clippy -D + fmt clean; shutdown 30/30 back-to-back, 20/20 under 14-core load). mped NO-GO->GO after in-thread doc-honesty fixes: rewrote docs/adr/003 (HIGH-1, present-tense "exits 0" lie), main.rs:249 self-exit comment (MEDIUM-1, bind failure no longer reaches that arm -> only a panic does), added a CHANGELOG bullet (LOW-3). Filed task-0241 (LOW-1: a server-task PANIC still exits 0 -> same systemd-Restart hazard, panic case).
FORWARD-CARRY: any future change to run_server must keep TCP failure non-fatal (opt-in transport must never kill the mandatory one). module.nix comment now says "task-0233 (done)".
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Opt-in TCP `--bind` failure now gracefully degrades to unix-only instead of taking the daemon down.

Problem: a runtime TCP --bind failure (e.g. EADDRINUSE) propagated out of run_server; main treated that as the task-0195 server-task self-exit -> clean exit 0. Under systemd Restart=on-failure (task-0204) exit 0 is not retried, so a busy opt-in port left a DEAD daemon reported as SUCCESS, taking the working unix socket down with it.

Change (commit 815d1b8): run_server catches the TCP bind error, logs a loud warning, and continues serving the mandatory unix control socket. A malformed --bind still fail-fasts at validate(); the unix socket bind failure stays fatal (it is the mandatory transport). The bind_failure shutdown test was rewritten to encode the degrade (daemon stays up, warns, still shuts down cleanly on SIGTERM) and the now-stale "TCP failure exits 0" claims in docs/adr/003, main.rs, nix/module.nix and CHANGELOG were corrected.

User/operator impact: a momentarily-busy opt-in TCP port no longer crashes the daemon or misreports a dead daemon as healthy; the unix socket keeps serving. Operators should alert on the "failed to bind ... unix control socket ONLY" warning (systemd will still show the unit active).

Tests: 319 workspace tests, clippy -D warnings, fmt clean; shutdown suite 30/30 back-to-back and 20/20 under 14-core load. Review: qa GO, mped GO (after doc-honesty fixes).

Follow-ups: task-0240 (daemon logs to stdout not stderr — decide/document), task-0241 (a server-task panic still exits 0 — same hazard for the panic case).
<!-- SECTION:FINAL_SUMMARY:END -->
