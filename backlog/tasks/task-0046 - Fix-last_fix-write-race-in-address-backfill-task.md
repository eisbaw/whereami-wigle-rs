---
id: TASK-0046
title: Fix last_fix write race in address-backfill task
status: Done
assignee:
  - '@myself'
created_date: '2026-05-10 10:51'
updated_date: '2026-07-04 23:15'
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
- [x] #1 Background reverse-geocode task does not write a stale (lat, lon, accuracy_m, sources) tuple to last_fix; only the address field is updated, OR the entire write happens while last_fix mutex is held
- [x] #2 Concurrent handle_locate + address backfill cannot interleave to produce an on-disk last_fix row whose fields disagree with the in-memory copy
- [x] #3 Test that exercises the race using tokio::join! of two operations on the same daemon state
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Extract the two real last_fix persistence operations so both retain the last_fix mutex through the DB write; expose a narrow persistence-closure seam for deterministic observation without a global test hook.
2. Add a multi-threaded regression on one DaemonState: seed OLD with equal timestamp, block DB on an OS thread, start OLD backfill, signal exactly at pre-DB persistence, assert last_fix remains locked, start NEW persist, release DB, tokio::join! both handles, and compare every memory/DB field.
3. Add bounded watchdogs/cleanup-before-assert so failures cannot hang; prove the old split-lock implementation fails the guard-retention assertion and task-0100 cannot mask it.
4. Run the focused test 10x, related last_fix tests, full nix develop -c just e2e, fmt, and diff check; leave AC#3 unchecked pending parallel review.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Fix: hold state.last_fix mutex (tokio) across the lock_db (std) DB write in both handle_locate and the address-backfill background task. Lock ordering is consistent across both sites (last_fix -> db) so no deadlock potential.

Why this closes the race: previously the bg task built a LastFixRow snapshot, dropped last_fix, then acquired lock_db. In the gap a concurrent handle_locate could run end-to-end, persist a newer row, then the bg task's stale-but-now-with-address row overwrote it. Same race shape existed in handle_locate itself if two concurrent calls interleaved their in-memory and DB writes.

The DB writes are sync (no .await), so holding the std mutex inside the tokio mutex is fine. Both critical sections remain microsecond-class.

AC #3 (test exercising the race) was deferred — writing a deterministic concurrency test for this would require either (a) injecting a yield point inside set_last_fix, or (b) a stress test with many threads. The structural fix is auditable from the code alone; pinning it via test would require a test-only knob in the production path. Documented in code comments referencing task-0046.

Honesty correction (2026-07-05): this task was previously marked Done while AC #3 remained unchecked, and its notes explicitly deferred the required deterministic concurrency regression. That was an overclaim. The task is reopened; no completion claim will be made until the original AC #3 is implemented, independently reviewed, and verified.

Phase-3 honesty audit reopened this task: it was marked Done while AC#3 remained unchecked, and the prior notes explicitly deferred the required concurrency regression. Existing implementation intent is preserved, but completion was overclaimed until the test is implemented and independently reviewed.

Phase-3 completion work (2026-07-05): added a deterministic multi-threaded regression on one DaemonState. An OS thread holds the real DB mutex; the backfill signals immediately before its real SQLite write; the test verifies the last_fix guard is still held, then races the newer-fix persistence and joins both operations with tokio::join!. Equal timestamps prevent task-0100 monotonic UPSERT behavior from masking stale ordering. Cleanup precedes assertions and all waits are bounded. Mutation proof: moving drop(last) before persistence makes the test fail at the guard-retention assertion. Verification before review: focused test passed 10/10; all 10 last_fix-filtered tests passed; just e2e passed 328 tests plus strict clippy; just fmt and git diff --check passed.

Independent pre-commit gate (2026-07-05): qa-test-runner GO after focused 10/10 plus 328-test e2e, strict Clippy, fmt, and diff checks. mped-architect initially blocked the stale historical Final Summary; after it was replaced through backlog CLI, re-review returned GO with no code/test findings.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed the last_fix address-backfill race and pinned it with a deterministic regression. Both address backfill and new-fix persistence retain the last_fix mutex through the real SQLite write using one consistent last_fix -> db lock order. The multi-threaded test blocks the real DB boundary, races both operations on one DaemonState with tokio::join!, uses equal timestamps so monotonic UPSERT cannot mask ordering, and verifies every persisted field agrees between memory and disk. Mutation testing confirmed the old split-lock ordering fails the guard-retention assertion. Pre-review verification passed the focused test 10/10, all last_fix tests, 328-test just e2e plus strict Clippy, formatting, and diff checks.
<!-- SECTION:FINAL_SUMMARY:END -->
