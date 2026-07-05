---
id: TASK-0201
title: Bound concurrent connections and throttle the O(n^2) history path (local DoS)
status: Done
assignee: []
created_date: '2026-07-02 04:52'
updated_date: '2026-07-05 03:03'
labels:
  - security
  - performance
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The accept loop spawns a handler per connection with no semaphore/cap. Each history request re-runs segment_fixes which recomputes geometric_median over the growing running cluster once per fix plus again in flush - O(n^2) x Weiszfeld-iterations synchronous work on a tokio worker with no rate limit. A local client can open unbounded connections each firing history over the full retention window to saturate worker threads and stop locate/scan. Ref whereamid/src/server.rs:316-331,943-955, history.rs:97-115. Complements task-0087 (amortize) with the DoS/bound angle.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 concurrent handlers are capped at a configurable limit; excess connections queue or are rejected rather than spawning unboundedly
- [x] #2 a burst of concurrent history requests does not make locate unresponsive beyond a bounded latency
- [x] #3 segmentation cost is bounded (no per-fix full recompute, or run on spawn_blocking)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a positive --max-connections option (default 32) with README/PRD/Nix option, ExecStart, and flag-parity coverage; acquire one shared Unix/TCP permit before accept and move it into the tracked handler future.
2. Add a one-slot history admission gate that rejects excess history before the DB query. Retain an Arc-owned lease across both the blocking CPU closure and socket response writing.
3. Move cancellation-aware segmentation and JSON serialization into one spawn_blocking job that captures no DaemonState/DbHandle; signal local cancellation on async-handler Drop and honor daemon shutdown.
4. Add deterministic tests for cap queueing/refusal release, history-burst busy responses plus responsive locate/timer, blocking-job permit lifetime, and shutdown cleanup.
5. Run full Nix e2e and mandatory parallel QA/architecture gates before commit; document the remaining O(N) range-query/allocation limit honestly.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0102/0190: per-connection handlers in server.rs run_server() are still DETACHED tokio::spawns (a comment there now points here). DaemonState::spawn_tracked(&self, fut) now exists — if you want connections drained/counted at shutdown, route them through it (it reaps completed slots and refuses after cancel). But note spawn_tracked uses an UNBOUNDED JoinSet, so for connection *bounding* youd want a semaphore/cap in front of accept(), not just tracking. The shutdown drain (run_shutdown_drain in main.rs, 5s budget) currently does NOT wait for connection tasks by design.

Forward update from TASK-0192 (2026-07-05): per-connection handlers are no longer detached. server.rs now routes them through DaemonState::spawn_tracked so shutdown owns/reaps them before SQLite actor Stop. The JoinSet remains unbounded, so this task still owns admission caps/semaphore behavior. Because timeout cleanup must reap tracked futures for quota/DB lifecycle safety, the synchronous O(n^2) segment_fixes + JSON phase can still extend shutdown and saturate workers; bound/offload/amortize that CPU work here (with TASK-0087) rather than assuming connection tasks are outside the drain.

Forward update from TASK-0087 (2026-07-05): the per-fix geometric-median recomputation is fixed before this task. History membership now uses an O(1) running spherical mean and each retained segment runs one cancellation-aware detailed geometric median at flush, so total segmentation work is O(N times bounded Weiszfeld iterations). TASK-0201 still owns the configurable shared connection cap, reject-not-queue history single-flight before the DB query, spawn_blocking for segmentation plus JSON, permit ownership through response writing, and prompt shutdown cancellation. The range query/result allocation remains O(N) and is not an absolute per-request memory bound.

Implemented a configurable shared Unix/TCP connection cap (--max-connections, default 32, enforced range 1..=4096) with admission before accept so excess clients remain in the kernel backlog and no permit-waiter tasks are spawned. Added a reject-not-queue single-flight history gate acquired before SQLite; the lease is retained by the actor command, cancellable spawn_blocking segmentation/JSON work, and the socket response envelope. Cancellation now bridges async Drop and daemon shutdown into TASK-0087 segmentation. Added exact queued/active DB cancellation, real stalled socket write, shared-transport cap, responsive locate/timer under history burst, blocking-worker abort, and production shutdown-drain regressions. Remaining limitation is documented honestly: one admitted history range query/result/response still allocates O(N) memory.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Bounded control-channel concurrency and isolated history work from the async runtime. Both mandatory QA and architecture gates returned GO after the four initial review blockers were fixed. Final validation before tracker completion passed 417 debug tests, 417 release tests, strict all-target clippy, formatting, Nix option-bound evaluation/build for 4096 vs 4097, and exact 24-flag Nix/CLI parity; QA additionally passed 400 repeated adversarial boundary executions.
<!-- SECTION:FINAL_SUMMARY:END -->
