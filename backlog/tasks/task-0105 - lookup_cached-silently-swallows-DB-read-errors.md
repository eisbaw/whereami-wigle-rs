---
id: TASK-0105
title: lookup_cached silently swallows DB read errors
status: To Do
assignee: []
created_date: '2026-05-20 21:11'
labels:
  - robustness
  - observability
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/resolver.rs:131-148. lookup_cached pattern-matches Ok(Some(ap)) / Ok(None) / Err(e) where the Err path only warn-logs and continues. A real DB error (corruption, disk full, lock contention) returns the same shape as 'not in cache'. handle_locate then treats the BSSID as a miss, kicks off background resolver, burns API quota refetching what's actually still in the DB. db_write_failures counts writes, not reads, so this is invisible to ops.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add db_read_failures: u64 counter to DaemonState.stats (parallel to db_write_failures); expose via /stats
- [ ] #2 lookup_cached increments db_read_failures on Err, OR returns a typed result the caller can inspect
- [ ] #3 Unit test: simulate a DB lock-busy or query error; assert the counter increments
- [ ] #4 Whereami stats CLI line shows the new counter
<!-- AC:END -->
