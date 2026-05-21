---
id: TASK-0109
title: Enforce last_fix → db mutex acquisition order in types
status: To Do
assignee: []
created_date: '2026-05-20 21:12'
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
