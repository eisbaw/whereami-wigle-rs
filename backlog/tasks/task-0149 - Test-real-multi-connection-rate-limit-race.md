---
id: TASK-0149
title: 'Test: real multi-connection rate-limit race'
status: To Do
assignee: []
created_date: '2026-05-20 21:29'
labels:
  - tests
  - concurrency
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P0-2. db.rs:1278 test_rate_limiting_concurrent uses Arc<Mutex<Database>>; the outer Mutex serializes 64 threads. Removing the SQL transaction's commit would still pass — the test proves the Mutex, not the SQL transaction. Moving to connection-per-task in future would silently regress quota.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Open the DB on disk (sqlite WAL); spawn N=8 threads each holding its OWN rusqlite::Connection to the same file
- [ ] #2 Race try_reserve_api_call across them; assert total reservations == daily_limit exactly (no overshoot, no undershoot)
- [ ] #3 Test failure mode: comment out the SQL BEGIN IMMEDIATE in try_reserve_api_call temporarily; verify the test FAILS (proves the SQL transaction is what's enforcing correctness)
<!-- AC:END -->
