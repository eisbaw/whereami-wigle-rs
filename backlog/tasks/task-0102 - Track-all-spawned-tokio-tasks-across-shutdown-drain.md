---
id: TASK-0102
title: Track all spawned tokio tasks across shutdown drain
status: Done
assignee:
  - '@claude'
created_date: '2026-05-20 21:11'
updated_date: '2026-07-02 22:43'
labels:
  - robustness
  - shutdown
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/main.rs:236-240 + src/server.rs:494-499 + src/server.rs:511-561. Cooperative shutdown (task-0075) drains the four named loops (scan/drain/prune/server) but every scan-loop iteration spawns a fresh resolve_background task, and handle_locate spawns address-resolution + a fallback resolve_background. These spawns are detached. On SIGTERM the daemon waits 5s for the four named loops; orphaned spawns mid-API-call are silently killed when the process exits, leaving partial writes (e.g. WiGLE quota reserved but no aps row).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Detached spawns moved behind a tokio::task::JoinSet (or similar handle-tracking structure) owned by DaemonState
- [x] #2 Shutdown drain waits up to 5s for the JoinSet to drain before forced-exit (matches the existing budget from task-0075)
- [x] #3 Stats counter orphan_tasks_aborted increments on each forced-cancellation at shutdown
- [x] #4 Test: spawn 5 fake resolve_background tasks that sleep 10s, trigger shutdown after 1s, assert orphan_tasks_aborted == 5 OR the drain waited (whichever the implementation chose)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
DaemonState gains tasks: Mutex<JoinSet<()>> + orphan_tasks_aborted: AtomicU64 and a spawn_tracked(&self, fut) helper (refuses after cancel; reaps completed slots via try_join_next). Convert the 3 detached spawns (scan-loop resolve, handle_locate fallback resolve, nominatim backfill) to spawn_tracked; per-connection spawn stays detached (task-0201). main mem::takes the JoinSet and drains it in run_shutdown_drain within the 5s budget; survivors force-aborted and counted into orphan_tasks_aborted, surfaced in StatsResponse + protocol.md. Paused-time test: 5x10s tracked tasks, cancel after 1s -> orphan==5.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented in commit 414a048 (spawn_tracked + stat) on top of the drain from 62d7957.

- DaemonState gained `tasks: std::sync::Mutex<tokio::task::JoinSet<()>>` and `orphan_tasks_aborted: AtomicU64`.
- spawn_tracked(&self, fut): reaps completed slots via `while set.try_join_next().is_some() {}` on every call (a JoinSet only frees a completed slot when joined, else it grows unbounded), and REFUSES to spawn once is_cancelled() (structural "no new work after shutdown", shared with task-0190 AC#2).
- Converted the 3 detached spawns: scan-loop proactive resolve (main.rs), handle_locate fallback resolve, and the Nominatim address backfill (server.rs). The per-connection accept spawn stays detached ON PURPOSE (bounded by READ/WRITE timeouts) with a comment pointing at task-0201.
- main mem::takes the JoinSet out of the std::sync::Mutex (cannot hold that guard across await) and drains it in run_shutdown_drain within the 5s budget; survivors are abort_all()d and counted.

Gotcha: JoinSet<()> tasks capture Arc<DaemonState>; the JoinHandle in the set does not, and the task future (with its Arc) is dropped by the runtime on completion, so no DaemonState reference cycle leaks — only a tiny () slot lingers until the next spawn_tracked reaps it.

Review gate (orchestrator, 2026-07-03): qa-test-runner GO (259 tests green, clippy -D warnings clean, fmt clean, shutdown.rs stable across 4 runs ~0.34s vs 4s budget; paused-clock tests 3x stable). mped-architect NO-GO scoped to AC#3: orphan_tasks_aborted was write-only — bumped only after the server socket closed and reset to 0 at startup, so "surfaced via stats" was unobservable-by-construction (comment-vs-code overclaim class). FIXED in-thread, commit bd0cb9f: count persisted to metadata key orphans_aborted_last_shutdown at clean shutdown (0 included) and rehydrated at startup, so stats reports the LAST clean shutdown; count also logged at shutdown; timeout path now reaps finished-but-unjoined tracked tasks before counting (review C1 overcount); docs/protocol.md + StatsResponse doc corrected to real semantics incl. named loops (review H2). Post-fix gate: 264 tests green (bin 139->143), clippy clean, fmt clean. AC#3 now genuinely met.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Track the three detached background spawns in a DaemonState-owned JoinSet so shutdown drains them under one budget instead of killing them mid-API-call.

What changed:
- Added DaemonState.tasks (Mutex<JoinSet<()>>) + spawn_tracked(&self, fut), which reaps finished slots each call and refuses after cancel.
- Routed scan-loop resolve, locate fallback resolve, and Nominatim backfill through spawn_tracked. Per-connection handlers stay detached (task-0201).
- run_shutdown_drain waits up to 5s for the JoinSet (+ named loops), then force-aborts survivors and increments orphan_tasks_aborted, surfaced in StatsResponse + handle_stats and documented in docs/protocol.md.

Why: orphaned resolve/backfill tasks killed at process exit left partial writes (e.g. a reserved WiGLE slot with no aps row).

Tests: drain_aborts_five_stuck_tracked_tasks (AC#4 shape: 5 tasks sleep 10s, 1s budget, assert orphan==5); drain_counts_only_survivors; spawn_tracked_runs_the_future / spawn_tracked_refuses_after_cancel.

Risks/follow-ups: bounded mpsc backpressure for resolve_background is still task-0121; consolidating the now-5 duplicated test DaemonState literals is filed as task-0230.
<!-- SECTION:FINAL_SUMMARY:END -->
