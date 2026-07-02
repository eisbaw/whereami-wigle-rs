---
id: TASK-0201
title: Bound concurrent connections and throttle the O(n^2) history path (local DoS)
status: To Do
assignee: []
created_date: '2026-07-02 04:52'
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
