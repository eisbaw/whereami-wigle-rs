---
id: TASK-0254
title: Unit-test the handle_stats and debug_statuses db_read_failures increments
status: To Do
assignee: []
created_date: '2026-07-08 18:06'
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
