---
id: TASK-0254
title: Unit-test the handle_stats and debug_statuses db_read_failures increments
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 18:06'
updated_date: '2026-07-08 18:12'
labels:
  - test-coverage
  - observability
dependencies:
  - TASK-0252
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0252 review (mped-architect LOW). task-0252 wired record_db_read_failure() into handle_stats (db.stats() Err) and handle_debug (debug_statuses Err), but those two reporting-surface increments are NOT unit-tested (the 3 tests added cover resolve_chain skip_cached/skip_not_found and the pending drain). Both are trivial one-line mirrors, but the residual risk is exactly the silent-sibling class this counter exists to fight: a future refactor could drop either increment with no failing test. Add cheap deterministic tests: drop the relevant table(s) so db.stats() / debug_statuses return Err, drive handle_stats / handle_debug, and assert state.db_read_failures incremented. Mirror the DROP TABLE + assert-counter idiom from resolver::tests::resolve_chain_skip_cached_read_error_increments_db_read_failures. Assert on the counter, not log capture (task-0248 tracing-capture is flaky).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Deterministic unit test: handle_stats increments db_read_failures when db.stats() read errors
- [x] #2 Deterministic unit test: handle_debug increments db_read_failures when debug_statuses read errors
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed the silent-sibling test gap from task-0252. The two reporting-surface db_read_failures increments (handle_stats db.stats() Err, handle_debug debug_statuses Err) were counted but untested; a future refactor could have dropped either with no failing test.

Added two deterministic regressions (server.rs stats_tests): each DROPs the aps table to force a real read Err, then asserts db_read_failures goes 0->1. handle_stats also asserts the faulting response reflects the increment (post-stats snapshot, task-0252 hardening) and that row counts fall back to 0. The handle_debug test seeds a scan so debug_statuses has an input BSSID to look up. Both bite (removing the increment fails the ==1 assert).

Verification: pure test-only change. Full gate green in-thread — whereamid bin 449 tests (447+2), workspace 740 total, 0 failed; clippy -D warnings clean; fmt clean. Per the loop's "don't spawn a subagent per trivial task", this test-only cycle was verified by the full gate + bite-analysis with the orchestrator as reviewer-of-record rather than a separate dual-subagent review.
<!-- SECTION:FINAL_SUMMARY:END -->
