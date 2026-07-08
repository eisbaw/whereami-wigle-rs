---
id: TASK-0109
title: Enforce last_fix → db mutex acquisition order in types
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:12'
updated_date: '2026-07-08 21:07'
labels:
  - refactor
  - concurrency
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:497-561 + :585-617. handle_locate and the address-backfill task acquire last_fix (tokio) then db (std). The comment at server.rs:540-550 documents this but nothing prevents a future contributor from swapping the order. 5 sites take both locks. One accidental swap = real deadlock invisible to tests because they don't race both async tasks. Compile-time enforcement via DaemonState::with_last_fix_and_db(|fix, db| ...) helper lifts the invariant from comment to type.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add DaemonState::with_last_fix_and_db<R>(closure: FnOnce(&mut LastFix, &mut Connection) -> Result<R>) helper
- [ ] #2 All current sites taking both locks migrate to use it
- [ ] #3 Both poison-recovery branches handled inside the helper (one place, not five)
- [ ] #4 Test: a synthetic test that tries to acquire the locks in reverse order can be constructed only if the public API permits it; the helper closes that door
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Triaged 2026-07-08: PREMISE SUPERSEDED by the DbHandle async-actor architecture (post-dates this 2026-05-20 task). VERIFIED: (1) every production last_fix.lock().await site (server.rs) is followed by an ASYNC state.db.<op>().await (DbHandle actor message-passing), NOT a std Mutex/Connection lock; (2) lock_db (the only path to a rusqlite Connection) is #[cfg(test)] — production has NO &mut Connection to co-acquire with last_fix. So the two-std-mutex reverse-order deadlock this task targets CANNOT be constructed in production (AC#4's goal — reverse-order deadlock unrepresentable — is already met STRUCTURALLY by making db access async), and AC#1's with_last_fix_and_db(&mut LastFix, &mut Connection) helper is inapplicable (no Connection exists). The residual real invariant — hold the last_fix tokio guard ACROSS the async db op for atomicity — is owned and regression-tested by task-0046 (last_fix_concurrency_tests::concurrent_backfill_and_new_fix_keep_memory_and_db_consistent, server.rs:3347, with the explicit "old task-0046 race" assertion at :3450).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
CLOSED AS SUPERSEDED (no code change). This task (2026-05-20) assumed handle_locate + address-backfill acquire two STD mutexes — last_fix (tokio) then db (std Connection) — and proposed a with_last_fix_and_db(&mut LastFix, &mut Connection) helper to make the reverse-order deadlock unrepresentable. A later architectural change (the DbHandle async-actor db layer) obsoleted that premise: production db access is now async message-passing (state.db.<op>().await), there is NO production std Connection (lock_db is #[cfg(test)] only), and thus NO two-std-mutex reverse-order deadlock is possible — the door AC#4 wanted closed is already closed STRUCTURALLY by the architecture. AC#1's &mut Connection helper is inapplicable. The one residual invariant (hold the last_fix tokio guard across the async db op for write-atomicity) is already lifted-and-tested by task-0046 (last_fix_concurrency_tests). ACs left UNCHECKED (not implemented — they are moot under the current architecture; not ticked, to avoid AC-gaming). Verification: grep-confirmed no std db mutex co-held with last_fix in any production path; lock_db is test-only (39 uses, all #[cfg(test)]); task-0046 concurrency test present + asserting the atomicity. Recommend: if a future refactor ever reintroduces a synchronous Connection on the last_fix path, RE-OPEN this. No follow-up filed.
<!-- SECTION:FINAL_SUMMARY:END -->
