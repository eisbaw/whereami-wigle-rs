---
id: TASK-0110
title: 'not_found table: add eviction policy / upper bound'
status: To Do
assignee: []
created_date: '2026-05-20 21:12'
labels:
  - robustness
  - db
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/db.rs:512-536. is_not_found treats anything within ttl_days as known-absent. delete_not_found is called only during revival in pending.rs, bounded to 5 per drain. A long-running daemon at a busy public location (cafe, train station) accumulates not_found rows for every transient device MAC ever seen — table grows monotonically. db_size_bytes is in stats but no per-table sizing. No prune step.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Schema migration v5->v6 OR a periodic prune in pending drain that DELETEs not_found rows older than not_found_ttl_days × 2 (a soft cap; the row would never be re-checked anyway after ttl)
- [ ] #2 Optional: --not-found-max-rows N cap; OLDEST rows evicted when count exceeds N
- [ ] #3 Stats: not_found_count exposed via /stats (parallel to ap_count)
- [ ] #4 Test: insert 1000 not_found rows older than 2*ttl, run prune, assert table has 0 rows
<!-- AC:END -->
