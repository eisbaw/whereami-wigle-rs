---
id: TASK-0237
title: >-
  De-flake shutdown.rs sigterm test: readiness handshake instead of 300ms sleep;
  close the real early-SIGTERM window
status: Done
assignee:
  - '@mark'
created_date: '2026-07-03 09:55'
updated_date: '2026-07-04 16:49'
labels:
  - testing
  - daemon
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
QA gate finding (cycle 10, H1, pre-existing — not introduced by 0204-0208): whereamid/tests/shutdown.rs sigterm_exits_within_drain_budget sleeps a fixed 300ms after spawning the daemon before sending SIGTERM (shutdown.rs:162). If SIGTERM lands before main installs the handler, the process dies by default signal disposition (wait_status 15) instead of draining, and the test asserts exit 0 -> fail. Observed 1 failure in 4 runs under ambient load (no concurrent build); 3/3 green isolated; green in the hermetic nix test check. This intermittently blocks the commit gate (just e2e). Fix direction from QA: (a) test side — replace the fixed sleep with a readiness handshake: poll for the daemon's ready signal (the socket file appearing / a ready log line / a version round-trip) before signalling; (b) daemon side — consider that the window is REAL, not just a test artifact: between exec and signal-handler install, a systemctl-stop SIGTERM kills the daemon without a drain. Evaluate installing the SIGTERM handler earlier in main (before DB open/migrations) so the drain path covers startup, or document why the window is acceptable. Both sides, or an explicit decision on (b), are in scope.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 the sigterm test passes 20/20 consecutive runs under parallel load (e.g. while a cargo build runs)
- [x] #2 no fixed startup sleep remains in shutdown.rs; readiness is event-based
- [x] #3 the early-SIGTERM window is either closed (handler installed before slow startup work) or explicitly documented as accepted with reasoning
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Daemon (AC#3): pre-register SIGTERM+SIGINT Signal streams right after args.validate() (main.rs ~55), before Database::open/migrations. tokio signal() installs the OS handler at call time, so a signal during startup is buffered (not default-disposition-fatal) and drained at the existing select!. Replace signal::ctrl_c() in the select! with the pre-registered SIGINT stream so both windows close.
2. Test (AC#2): in shutdown.rs sigterm_exits_within_drain_budget, replace the fixed 300ms sleep + try_wait assert with an event-based readiness poll on the --socket file existing (post-reorder: socket-file-exists => handler installed; log-verbosity-independent), with a ~10s budget + small poll interval, surfacing early child-exit via try_wait+stderr.
3. Verify (AC#1): run the sigterm test 20x consecutively under a concurrent cargo build; require 20/20.
4. Gate: nix develop -c just e2e + nix develop -c just fmt. Confirm bind_failure/live_socket shutdown tests still pass (select! arms unchanged). Commit (no AI attribution); tracker in a separate backlog: commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ID-RECYCLE NOTE: this id previously belonged to a now-archived duplicate of task-0233 (TCP-bind exit-0 residual), filed and archived in cycle 10. This task (sigterm de-flake) is unrelated to that one.

Implemented (commit d8f35fb):
- Root cause: the SIGTERM handler was installed at main.rs (old) AFTER Database::open + migrations + socket bind, so a SIGTERM during startup hit the default disposition and terminated the daemon by signal 15. The old test masked this with a fixed 300ms sleep, which is far too short under CPU contention. Measured 1/20 pass under 14-core saturation (the original report was 1/4 under ambient load — load severity matters).
- Fix (daemon, AC#3): register SIGTERM+SIGINT streams right after args.validate(), before any slow startup work. tokio signal() arms the OS handler at call time and latches a pending signal in a watch channel (verified from tokio-1.52.1 vendored source during review), so a startup-phase signal is deferred and drained on the normal path once startup completes — never a default-disposition kill. Also closed the SIGINT window (was signal::ctrl_c() at the select!) for symmetry.
- Fix (test, AC#2): replaced the fixed 300ms sleep with wait_until_socket_ready(), a readiness poll on the --socket file appearing; fails fast if the daemon exits during startup. Sound ONLY because the reorder puts handler-install strictly before the bind.
- Review hardening (in-thread, same commit): widened the post-SIGTERM exit wait 4s->6s (above the 5s drain budget) so a slow-but-correct drain is reported by the <4s health assertion, not a misleading timeout; reworded comments per the mped honesty review (no "drain covers startup" overclaim; "signal 15" not "exit 15"; "readiness poll" not literally "event-based").
Verification: old 1/20, new 20/20 (+10/10 after hardening) under 14-core load; full gate 312 workspace tests + clippy -D warnings + fmt clean. Review gate: qa-test-runner GO, mped-architect GO.
GOTCHAS / forward-carry: (1) The socket-file-exists readiness proxy is valid ONLY while handler-install precedes the socket bind in main.rs — do not reorder the bind before the signal() calls or the test races again. (2) A SIGTERM during a slow migration is DEFERRED until startup completes (not immediate); fine for tiny migrations, within systemd stop timeout. (3) An inherent sub-microsecond window remains before the two signal() calls (during arg parse) — un-closable (no handler before the process exists) and un-hittable by the test; documented in the code comment.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
De-flaked the whereamid shutdown.rs sigterm test and closed a real early-SIGTERM window in the daemon.

Problem: whereamid installed its SIGTERM handler only just before the shutdown select!, i.e. AFTER DB open + migrations and the socket bind. A SIGTERM during that startup window (e.g. `systemctl stop` mid-migration) killed the daemon by default disposition (signal 15) instead of draining. The integration test papered over this with a fixed 300ms sleep before signalling, which under CPU load was too short — it failed 19/20 under 14-core saturation.

Changes (commit d8f35fb):
- whereamid/src/main.rs: install the SIGTERM and SIGINT signal streams immediately after args.validate(), before any slow startup work; consume them in the existing select! (the old signal::ctrl_c() SIGINT arm is replaced by the pre-registered stream). tokio arms the OS handler at signal() call time and latches a pending signal, so a startup-phase signal is deferred and handled by the normal drain once startup completes.
- whereamid/tests/shutdown.rs: replace the 300ms sleep with wait_until_socket_ready() (readiness poll on the control socket appearing, sound because the handler is now installed before the bind); widen the post-SIGTERM exit wait 4s->6s so a slow-but-correct drain is diagnosed by the <4s health assertion rather than a misleading timeout.

User impact: `systemctl stop` (or any SIGTERM) during daemon startup now shuts down gracefully instead of a hard kill; CI/commit gate (just e2e) no longer flakes on this test.

Tests: sigterm test 20/20 under 14-core load (old code 1/20 under the same harness — reproduced by stashing the fix); full workspace 312 tests, clippy -D warnings, fmt all clean. Review gate: qa-test-runner GO, mped-architect GO (four non-blocking hardening findings applied in-thread).

Risks/follow-ups: a SIGTERM during a slow migration is deferred until startup completes (fine for current tiny migrations; forward-carried a note to task-0192 in case migrations move off the runtime). An inherent, un-hittable sub-microsecond window remains during arg parsing before the signal() calls (documented in code).
<!-- SECTION:FINAL_SUMMARY:END -->
