---
id: TASK-0119
title: Split DaemonState into subsystem structs
status: To Do
assignee: []
created_date: '2026-05-20 21:15'
labels:
  - refactor
  - architecture
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:77-107. DaemonState is a 10-field god-struct mixing tokio mutexes, std mutexes, async clients, in-memory caches, and stats counters. The lock-acquisition invariant is prose-documented (task-0109 addresses this). inflight is conceptually owned by resolver.rs but lives next to network clients on DaemonState. stats reaches across boundaries (server.rs:967-989) for counters that should live in subsystem structs.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Split into three sub-structs: Persistence { db, last_fix, db_write_failures, db_read_failures }, Resolution { wigle, apple, inflight, debouncer, http_timeouts }, Geocoding { nominatim, address_cache }
- [ ] #2 DaemonState becomes a thin holder of the three plus args/shutdown
- [ ] #3 Persistence exposes with_last_fix_and_db(closure) (subsumes task-0109)
- [ ] #4 Each subsystem's stats fields rolled up into the /stats response via a method on the subsystem (not field access from server.rs)
- [ ] #5 All existing tests pass; the locking story is now testable per-subsystem in isolation
<!-- AC:END -->
