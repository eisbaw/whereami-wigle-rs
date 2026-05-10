---
id: TASK-0046
title: Fix last_fix write race in address-backfill task
status: To Do
assignee: []
created_date: '2026-05-10 10:51'
labels:
  - bug
  - server
  - concurrency
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
server.rs:469-498 reads state.last_fix under the tokio mutex, drops the lock, then writes the snapshot to disk via lock_db. Between drop and DB write a concurrent handle_locate can have already persisted a newer row. The background task then overwrites that newer row with stale lat/lon plus a freshly-resolved address that no longer matches. Found in v0.4.0 review (mped-architect, swe-gardener).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Background reverse-geocode task does not write a stale (lat, lon, accuracy_m, sources) tuple to last_fix; only the address field is updated, OR the entire write happens while last_fix mutex is held
- [ ] #2 Concurrent handle_locate + address backfill cannot interleave to produce an on-disk last_fix row whose fields disagree with the in-memory copy
- [ ] #3 Test that exercises the race using tokio::join! of two operations on the same daemon state
<!-- AC:END -->
