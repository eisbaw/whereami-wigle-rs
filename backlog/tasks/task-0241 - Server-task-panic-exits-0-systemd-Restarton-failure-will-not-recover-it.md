---
id: TASK-0241
title: Server-task panic exits 0; systemd Restart=on-failure will not recover it
status: Done
assignee:
  - '@myself'
created_date: '2026-07-04 17:43'
updated_date: '2026-07-04 22:53'
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
- [x] #1 an unexpected server-task termination (unexpected Ok, or a JoinError/panic) causes a non-zero process exit
- [x] #2 a normal SIGTERM/SIGINT shutdown still exits 0 (the graceful path is unaffected)
- [x] #3 a test covers the server-task-fails-exits-nonzero path, or documents why it cannot be triggered deterministically
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Introduce a small internal shutdown-trigger/result classification seam in main.rs that preserves the task-0195 single-poll XOR: select the server JoinHandle result exactly once, classify every unexpected completion (Ok(Ok(())), Ok(Err), or JoinError/panic) as fatal, and prioritize the server branch over a simultaneously-ready signal.
2. Preserve graceful signal shutdown: abort and drain the still-unconsumed server handle exactly once, clean up the socket, then return Ok. For a consumed server handle, run the same orderly cancellation/drain/socket cleanup and return an error so the process exits non-zero.
3. Keep task-0233 behavior intact: optional TCP bind failures continue degrading to unix-only; unix listener setup remains fail-fast before task spawn.
4. Add deterministic unit regressions for clean unexpected completion, run_server-style Err, a real Tokio JoinError from panic, signal success, and failure propagation through the final main result. Keep the real-binary SIGTERM integration test as the exit-0 control; document why safely inducing an external accept-loop panic is not deterministic.
5. Update stale comments and the Unreleased changelog entry. Run focused shutdown/failure tests at least twice, then nix develop -c just e2e, nix develop -c just fmt, and git diff --check. Leave uncommitted and In Progress for the mandatory parallel QA/architecture review.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented the shutdown-result state machine without a production fault-injection hook. The server JoinHandle now retains run_server's Result; a biased select checks it before signals so a simultaneously-ready panic cannot be misclassified as a clean signal shutdown. Unexpected Ok(Ok), Ok(Err), and JoinError/panic are converted to a fatal error, then carried through the existing cancellation, background drain, metadata persistence, and socket cleanup before main returns Err/non-zero. On SIGINT/SIGTERM the still-pending server handle is aborted and consumed exactly once before the remaining named drain; on server failure the select already consumed it and it is never polled again (task-0195 XOR preserved). TASK-0233 TCP bind degrade remains unchanged and is pinned by regression runs.

Testing gotcha/rejected approach: the production accept loop is intentionally infinite, catches accept errors, and has no safe external input that deterministically panics it. Adding a hidden CLI/env crash flag solely for a process test would be a production workaround. Instead, a small internal classification/final-result seam is tested for unexpected clean completion, returned server Err, and a real Tokio panic JoinError; the existing real-binary SIGTERM test remains the exit-0 control. The select uses biased server-first ordering to close the otherwise-uncovered simultaneous-ready race.

Measured pre-review gate: 4/4 focused server-exit policy tests passed; real-binary SIGTERM exit-0 passed 2/2; task-0233 TCP graceful-degrade passed 2/2. `nix develop -c just e2e` passed 326 tests (15+22+1+88+185+12+3), 0 failed/ignored, and strict clippy -D warnings was clean. `nix develop -c just fmt` and git diff --check were clean. CHANGELOG and nix/module.nix now describe the actual restart behavior. Leaving ACs unchecked and status In Progress for the mandatory independent qa-test-runner + mped-architect review.

First independent review gate: qa-test-runner GO, but mped-architect NO-GO (M1, correctly blocking). Root cause: the signal branch aborted then awaited the server handle outside run_shutdown_drain; Tokio abort is cooperative, so non-yielding/blocking work could make SIGTERM unbounded and prevent cleanup, regressing task-0190 despite the happy-path process test.

M1 fixed at root: run_shutdown_drain now directly owns an optional JoinHandle<Result<()>> alongside the differently typed named handles, under the same timeout; no wrapper task. It pre-extracts the server AbortHandle, awaits the server only inside the bounded graceful future, and counts/aborts a surviving server on timeout. Failure-path XOR remains: a handle consumed in select is passed as None and never repolled; signal path aborts then moves the still-unconsumed handle into the bounded drain. Added drain_bounds_non_cooperative_server_handle using already-started spawn_blocking work (abort cannot stop it) to prove the 50ms budget returns and counts the survivor. Post-fix focused gate: server-exit policy 8/8 across two runs, bounded-server regression 2/2, and real-binary SIGTERM exit-0 2/2. Awaiting full re-review before checking ACs or marking Done.

Second architecture re-review: production M1 fix accepted, but mped-architect correctly kept NO-GO on test discipline: the non-cooperative regression had no independent watchdog, so a regression in run_shutdown_drain could hang before releasing spawn_blocking; the module comment also still claimed all tests used paused time. Fixed by wrapping the 50ms drain in a separate 2s Tokio timeout, releasing the blocking worker before unwrapping/asserting, and documenting the one deliberate real-time test. Watchdog regression passed 10/10 after the fix. Final reviewer re-check pending.

Final post-hardening review gate: qa-test-runner GO with 327/327 tests, 0 failed/ignored, strict clippy -D warnings clean, fmt clean, and git diff --check clean. Focused final reruns: server-exit policy 8/8 (including real Tokio panic JoinError twice), non-cooperative watchdog 2/2 (~50-60ms), real-binary SIGTERM exit-0 2/2, and task-0233 TCP degrade 2/2. mped-architect final GO with no findings after verifying the outer watchdog 10/10, bounded optional server-handle drain, single-poll XOR, cleanup-before-Err, biased failure priority, and documentation honesty. Both earlier NO-GO findings were fixed in this task; no follow-up is required.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Make unexpected server-task termination visible to systemd as a failure without sacrificing orderly shutdown.

Changes:
- Preserve run_server's Result in the Tokio JoinHandle and classify unexpected clean return, returned error, and JoinError/panic as fatal.
- Use server-first biased selection so a simultaneously-ready signal cannot mask a server failure as exit 0.
- Carry the fatal error through cancellation, bounded task drain, metadata persistence, and socket cleanup, then return Err/non-zero; normal SIGINT/SIGTERM still return Ok/0.
- Extend run_shutdown_drain to own the differently typed optional server handle under the existing timeout while preserving task-0195 single-consumption semantics.
- Add deterministic policy tests, a real Tokio panic JoinError case, and a watchdog-protected non-cooperative server-drain regression; update CHANGELOG and NixOS restart-policy comments.

Tests:
- Final independent QA: 327/327 tests passed; strict clippy -D warnings, fmt, and git diff --check clean.
- Focused: server-exit policy 8/8, watchdog 2/2 (plus architecture 10/10), real SIGTERM exit-0 2/2, TCP graceful-degrade 2/2.
- Final independent architecture review: GO, no findings.

Risk/limits:
- The production accept loop has no safe external input that deterministically panics it. Tests exercise the exact classification/final-result seam with a real Tokio panic JoinError instead of adding a production crash hook. No known follow-up remains.
<!-- SECTION:FINAL_SUMMARY:END -->
