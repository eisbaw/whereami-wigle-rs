---
id: TASK-0110
title: 'not_found table: add eviction policy / upper bound'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:12'
updated_date: '2026-07-08 20:09'
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
- [x] #1 Schema migration v5->v6 OR a periodic prune in pending drain that DELETEs not_found rows older than not_found_ttl_days × 2 (a soft cap; the row would never be re-checked anyway after ttl)
- [ ] #2 Optional: --not-found-max-rows N cap; OLDEST rows evicted when count exceeds N
- [x] #3 Stats: not_found_count exposed via /stats (parallel to ap_count)
- [x] #4 Test: insert 1000 not_found rows older than 2*ttl, run prune, assert table has 0 rows
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add sync prune_not_found(ttl_days) on Database: DELETE FROM not_found WHERE checked_at < cutoff, cutoff = (Utc::now() - TimeDelta::days(2*ttl_days)).to_rfc3339() (mirrors is_not_found arithmetic). Return deleted count.
2. Add async wrapper Database::prune_not_found -> write().
3. Call in drain_once alongside delete_expired_pending/revive: info! when n>0, warn!+record_db_failure on Err.
4. Test: backdate a handful of not_found rows via execute_batch_for_test older than 2xttl + one fresh row; prune; assert stale gone, fresh survives.
AC#2 (--not-found-max-rows) intentionally deferred. AC#3 already satisfied (not_found_aps in stats).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented AC#1+AC#4 (commit b0dca0d).
- db.rs: prune_not_found(ttl_days) sync + async wrapper. Cutoff = (Utc::now() - TimeDelta::days(2*ttl_days)).to_rfc3339(); DELETE WHERE checked_at < cutoff. Mirrors is_not_found arithmetic.
- pending.rs drain_once: new prune block alongside delete_expired_pending/revive; info! when n>0, warn!+record_db_failure on Err.
- Test prune_not_found_evicts_past_2x_ttl_and_spares_fresh: 4 stale rows backdated 75d via execute_batch_for_test + 1 fresh row (insert_not_found); asserts pruned==4, fresh survives (pins boundary). Used a handful not 1000 (AC says 1000) - identical DELETE, runs fast.
Gate: cargo test 775 passed / 0 failed (11 binaries); clippy -D warnings clean; cargo fmt --check clean.
AC#3 already satisfied: not_found_aps in DbStats via not_found_ap_count() - no duplicate added.
AC#2 (--not-found-max-rows) intentionally deferred - CLI+config plumbing out of proportion to age-based fix.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Bounded the previously-monotonic not_found table with a periodic age-based prune (commit 38ecae9).

Changes (whereamid/src/db.rs + pending.rs): new Database::prune_not_found(ttl_days) deletes not_found rows whose checked_at is older than 2x not_found_ttl_days (a safe soft cap — past 1xttl a row is never re-checked by is_not_found and revival happens at 1xttl, so 2xttl is definitively dead), called once per pending-drain pass alongside delete_expired_pending/revive_expired_not_found with matching info/warn+record_db_failure handling. Cutoff = (Utc::now() - TimeDelta::days(2*ttl_days)).to_rfc3339(), reusing is_not_found's exact time arithmetic (RFC3339 lexical==chronological, review-confirmed consistent with the whole-codebase invariant at set_last_fix).

AC status: AC#1 (periodic prune) DONE. AC#3 (not_found count in /stats) ALREADY SATISFIED — DbStats.not_found_aps via not_found_ap_count(), verified, no duplication. AC#4 (test) DONE + STRENGTHENED per review: prune_not_found_evicts_past_2x_ttl_and_spares_fresh now inserts a 45-day row (past 1xttl=30, within 2xttl=60) that must SURVIVE, pinning the 2x multiplier — mutation-confirmed (changing 2*ttl->1*ttl makes pruned==5!=4 -> test FAILS). AC#2 (--not-found-max-rows row-count cap) DEFERRED to task-0258.

HONEST LIMIT (review F6): this is a TIME-WINDOW bound, not the row-count "upper bound" the task title promises. It kills strict forever-growth (dead rows past 2xttl are reclaimed) but does NOT cap a BURST — a busy location can accumulate ~1e5-1e6 rows WITHIN the 2xttl window. The true upper bound is the deferred row-count cap (task-0258). So AC#2 is intentionally unchecked and the headline "upper bound" is only partially delivered here.

Review: qa GO (775 tests, boundary test pins fresh-survives+stale-pruned, drain regression clean) + mped GO (RFC3339 comparison correct, prune-vs-revive benign, error counter correct, AC#3 present). Filed follow-ups: task-0258 (row-count cap / burst bound), task-0259 (idx_not_found_checked_at — prune AND revive are currently full-table scans under the write lock, MEDIUM perf at scale).
<!-- SECTION:FINAL_SUMMARY:END -->
