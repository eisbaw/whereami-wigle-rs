---
id: TASK-0190
title: >-
  Graceful shutdown: notify_waiters lost-wakeup, drain timeout never aborts,
  WiGLE quota slot leak
status: To Do
assignee: []
created_date: '2026-07-02 04:50'
labels:
  - correctness
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The three background loops register shutdown.notified() only inside their select!. Notify::notify_waiters stores no permit, so a SIGTERM that lands while a loop is mid-iteration (scan 1.5-11s, drain up to 10 BSSIDs x 2 providers x 15s, prune) is lost: the loop re-parks and runs forever, even launching new scans/resolver tasks after shutdown began. main then always eats the full 5s drain timeout whose handler only logs - scan/pending/history handles are never .abort()ed despite the comment. Runtime-drop cancellation can kill drain_once between try_reserve_api_call (committed) and the refund, leaking a persisted quota slot for the day. Ref whereamid/src/main.rs:174-205,262-265, pending.rs:32-40.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 SIGTERM during a scan/drain/prune iteration exits the loop at the next await boundary without waiting out the sleep interval (use CancellationToken or watch)
- [ ] #2 no new scan or resolver work is started after the shutdown signal
- [ ] #3 the drain timeout path actually aborts remaining tasks; daemon exits within the drain budget in a mid-scan test
- [ ] #4 a quota slot reserved when shutdown interrupts a lookup is refunded or never leaks into the persisted counter
<!-- AC:END -->
