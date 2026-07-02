---
id: TASK-0121
title: Bounded mpsc for resolve_background spawns
status: To Do
assignee: []
created_date: '2026-05-20 21:17'
updated_date: '2026-07-02 22:23'
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
<!-- SECTION:NOTES:END -->
