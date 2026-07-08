---
id: TASK-0121
title: Bounded mpsc for resolve_background spawns
status: To Do
assignee: []
created_date: '2026-05-20 21:17'
updated_date: '2026-07-08 21:58'
labels:
  - refactor
  - concurrency
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/main.rs:236-240 (scan loop) + src/server.rs:494-499 (handle_locate cold-start). Every scan-loop tick + locate cold-start spawns a fresh tokio::spawn(resolve_background). InflightGuard dedup prevents per-BSSID overlap at the provider, but N concurrent tasks all sharing the same BSSIDs still pay the resolve_chain cost to discover the BSSIDs are deferred. Busy area + fast-scan = 5-10 outstanding tasks contending on the DB mutex.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Replace tokio::spawn(resolve_background(...)) with mpsc::Sender<Vec<String>> push from each spawn site
- [ ] #2 Single consumer task (started in main.rs alongside the four named loops) batches inputs, de-duplicates, and calls resolve_chain once per batch
- [ ] #3 Channel is bounded (capacity 32 default) with drop-oldest or back-pressure semantics — documented choice
- [ ] #4 Cooperative shutdown: consumer task selects on shutdown.notified() like the other loops
- [ ] #5 Stats counter: resolve_background_queue_depth exposed via /stats
- [ ] #6 Test: enqueue 100 calls with duplicate BSSIDs; assert resolve_chain was called once per unique BSSID
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0102: resolve_background is now spawned via DaemonState::spawn_tracked (scan loop + handle_locate fallback), which tracks each in an UNBOUNDED JoinSet and refuses after shutdown. This is tracking, NOT backpressure — a burst of newly-stable BSSIDs still spawns N concurrent resolves. A bounded mpsc/semaphore should sit INSIDE or IN FRONT of spawn_tracked (keep the shutdown-refusal + reap behavior). spawn_tracked reaps via try_join_next each call, so a bounded worker pool draining the JoinSet is a natural fit.

Triaged 2026-07-08 (NOT started — deferred to fresh context; NOT superseded, still real). The cycle-6 shutdown rework (spawn_tracked + JoinSet + run_shutdown_drain) addressed DRAINING background spawns at shutdown, but NOT the contention/redundancy this task targets (N outstanding resolve_background tasks from scan-loop ticks + locate cold-start all paying resolve_chain setup on overlapping BSSIDs; InflightGuard dedups only at the provider). spawn_tracked does NOT cap the COUNT of concurrent background resolves. So the premise holds.

Why deferred (deep, high blast radius): this is a concurrency-ARCHITECTURE change — replace tokio::spawn(resolve_background) at TWO sites (main.rs scan loop ~236, server.rs handle_locate cold-start ~494/1366 region via spawn_tracked) with an mpsc::Sender push; add a NEW single consumer task in main() alongside the 4 named loops that batches+dedups and calls resolve_chain once per batch; bounded channel (AC#3: choose+document drop-oldest vs back-pressure); cooperative shutdown via select! on shutdown.cancelled() INTEGRATED with run_shutdown_drain (the fragile area reworked across cycles 6/9/11 — the new consumer must drain its in-flight batch under the shutdown budget without racing the DB-actor stop); + a resolve_background_queue_depth stats field (mirror the db_read_failures wire pattern from task-0105/0252); + a dedup test (AC#6). Templates exist (the pending-drain loop for the shutdown-select shape; db_read_failures for the stats wire), so it is well-specified — but the shutdown-drain integration + back-pressure policy want a FRESH, non-fatigued context + the full dual review gate. Cross-ref cycles 6/9/11 + tasks 0102/0190/0195/0201/0237.
<!-- SECTION:NOTES:END -->
