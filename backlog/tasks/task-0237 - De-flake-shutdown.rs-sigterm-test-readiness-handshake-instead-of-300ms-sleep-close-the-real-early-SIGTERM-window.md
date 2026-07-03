---
id: TASK-0237
title: >-
  De-flake shutdown.rs sigterm test: readiness handshake instead of 300ms sleep;
  close the real early-SIGTERM window
status: To Do
assignee: []
created_date: '2026-07-03 09:55'
updated_date: '2026-07-03 09:56'
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
- [ ] #1 the sigterm test passes 20/20 consecutive runs under parallel load (e.g. while a cargo build runs)
- [ ] #2 no fixed startup sleep remains in shutdown.rs; readiness is event-based
- [ ] #3 the early-SIGTERM window is either closed (handler installed before slow startup work) or explicitly documented as accepted with reasoning
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ID-RECYCLE NOTE: this id previously belonged to a now-archived duplicate of task-0233 (TCP-bind exit-0 residual), filed and archived in cycle 10. This task (sigterm de-flake) is unrelated to that one.
<!-- SECTION:NOTES:END -->
