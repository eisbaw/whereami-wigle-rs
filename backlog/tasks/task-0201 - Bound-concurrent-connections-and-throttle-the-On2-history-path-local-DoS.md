---
id: TASK-0201
title: Bound concurrent connections and throttle the O(n^2) history path (local DoS)
status: To Do
assignee: []
created_date: '2026-07-02 04:52'
updated_date: '2026-07-05 01:29'
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
- [ ] #1 concurrent handlers are capped at a configurable limit; excess connections queue or are rejected rather than spawning unboundedly
- [ ] #2 a burst of concurrent history requests does not make locate unresponsive beyond a bounded latency
- [ ] #3 segmentation cost is bounded (no per-fix full recompute, or run on spawn_blocking)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0102/0190: per-connection handlers in server.rs run_server() are still DETACHED tokio::spawns (a comment there now points here). DaemonState::spawn_tracked(&self, fut) now exists — if you want connections drained/counted at shutdown, route them through it (it reaps completed slots and refuses after cancel). But note spawn_tracked uses an UNBOUNDED JoinSet, so for connection *bounding* youd want a semaphore/cap in front of accept(), not just tracking. The shutdown drain (run_shutdown_drain in main.rs, 5s budget) currently does NOT wait for connection tasks by design.

Forward update from TASK-0192 (2026-07-05): per-connection handlers are no longer detached. server.rs now routes them through DaemonState::spawn_tracked so shutdown owns/reaps them before SQLite actor Stop. The JoinSet remains unbounded, so this task still owns admission caps/semaphore behavior. Because timeout cleanup must reap tracked futures for quota/DB lifecycle safety, the synchronous O(n^2) segment_fixes + JSON phase can still extend shutdown and saturate workers; bound/offload/amortize that CPU work here (with TASK-0087) rather than assuming connection tasks are outside the drain.
<!-- SECTION:NOTES:END -->
