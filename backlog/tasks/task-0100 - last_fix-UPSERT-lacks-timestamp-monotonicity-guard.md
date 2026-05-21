---
id: TASK-0100
title: last_fix UPSERT lacks timestamp monotonicity guard
status: To Do
assignee: []
created_date: '2026-05-20 21:10'
labels:
  - correctness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/db.rs:815-836 + src/server.rs:599. set_last_fix calls INSERT...ON CONFLICT DO UPDATE without a 'WHERE excluded.at_rfc3339 >= last_fix.at_rfc3339' guard. Two concurrent handle_locate calls are serialized by the tokio mutex (task-0046), so in practice newer wins — but a clock skew event (NTP step backwards, container time fix on restart) would let an older fix overwrite a newer one. No DB-side constraint enforces monotonicity. Same for insert_fix on the fixes timeseries.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 set_last_fix UPDATE clause includes WHERE excluded.at_rfc3339 >= last_fix.at_rfc3339
- [ ] #2 Test: insert a fix at T2, then attempt set_last_fix with T1<T2; assert last_fix still reflects T2
- [ ] #3 Doc-comment on set_last_fix names the invariant: 'monotonic by at_rfc3339, older writes are silently dropped'
- [ ] #4 Optional: fixes table also gets a sanity check or comment that the ordering is by id (auto-incr), not at_rfc3339; no monotonicity guarantee in the timeseries (consider whether to add a CHECK)
<!-- AC:END -->
