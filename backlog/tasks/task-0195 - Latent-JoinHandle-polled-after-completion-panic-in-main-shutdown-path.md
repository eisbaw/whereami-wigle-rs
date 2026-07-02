---
id: TASK-0195
title: Latent JoinHandle-polled-after-completion panic in main shutdown path
status: Done
assignee:
  - '@claude'
created_date: '2026-07-02 04:51'
updated_date: '2026-07-02 22:22'
labels:
  - correctness
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The shutdown select has an arm '_ = &mut server_handle => {...}' that consumes the handle output; the drain block then awaits the same handle again (let _ = server_handle.await). tokio panics on a second poll after the output was taken. Today masked by the notify lost-wakeup bug (arm only fires on bind failure at t~0, and the drain times out on scan_handle.await first). Fixing the shutdown race, task-0143, or a mid-life server panic coinciding with parked loops arms it, turning 'server exited' into exit 101. Ref whereamid/src/main.rs:174-199.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 when the server task exits on its own (e.g. bind failure) the daemon shuts down cleanly with a non-panic exit
- [x] #2 no JoinHandle in main.rs can be polled twice after completion (structurally: Option-wrap or JoinSet)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Restructure main shutdown so the server JoinHandle is polled at most once: select arm sets a server_exited flag; only when NOT exited do we abort+push it into the drained handles. run_shutdown_drain awaits each named handle once and force-aborts via pre-extracted AbortHandles on timeout (never a 2nd poll). Integration test: bind-failure path exits clean, stderr has no "panicked".
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Fixed structurally as part of the shutdown redesign (commit 62d7957) and verified by integration test (commit 31c0491).

The select arm `res = &mut server_handle` now yields a bool `server_exited`. If true, the handle is spent (its output was consumed by the arm) and is dropped WITHOUT a second poll. If false (SIGINT/SIGTERM path), the handle is aborted and pushed into the drained `named` vec, where run_shutdown_drain awaits it exactly once; on drain timeout it is force-aborted via a pre-extracted AbortHandle, never re-awaited. So no JoinHandle can be polled after completion regardless of which shutdown trigger fires.

Lesson: `&mut handle` in a non-firing select arm only POLLS (safe, repeatable) — the panic is specifically re-polling after Ready took the output. The flag cleanly encodes the XOR.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Eliminate the latent double-poll panic in main shutdown: the server JoinHandle is now polled at most once.

The old code awaited server_handle in a select arm AND again in the drain; once the notify race or a bind-failure armed both, tokio panicked (exit 101) on the second poll. The select arm now returns a `server_exited` flag: on self-exit the spent handle is dropped unused; otherwise it is aborted and drained exactly once. Named loop handles are likewise awaited once in run_shutdown_drain and only ever force-aborted (never re-awaited) on timeout — structurally no handle is polled after completion.

Tests: integration test bind_failure_exits_cleanly_without_panic starts the daemon on an already-bound port; the server task returns and the daemon exits 0 with no "panicked" in stderr (would be exit 101 before the fix).

Risks: none beyond the timing nature of the integration test (15s generous budget; real exit is immediate).
<!-- SECTION:FINAL_SUMMARY:END -->
