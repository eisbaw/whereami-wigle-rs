---
id: TASK-0100
title: last_fix UPSERT lacks timestamp monotonicity guard
status: Done
assignee:
  - '@claude'
created_date: '2026-05-20 21:10'
updated_date: '2026-07-02 05:27'
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
- [x] #1 set_last_fix UPDATE clause includes WHERE excluded.at_rfc3339 >= last_fix.at_rfc3339
- [x] #2 Test: insert a fix at T2, then attempt set_last_fix with T1<T2; assert last_fix still reflects T2
- [x] #3 Doc-comment on set_last_fix names the invariant: 'monotonic by at_rfc3339, older writes are silently dropped'
- [x] #4 Optional: fixes table also gets a sanity check or comment that the ordering is by id (auto-incr), not at_rfc3339; no monotonicity guarantee in the timeseries (consider whether to add a CHECK)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add WHERE excluded.at_rfc3339 >= last_fix.at_rfc3339 to set_last_fix DO UPDATE.
2. Doc-comment invariant: monotonic by at_rfc3339, older writes silently dropped; note lexicographic==chronological holds for UTC to_rfc3339.
3. Comment on insert_fix: timeseries ordered by id (autoincr), no monotonicity guard.
4. Test: set_last_fix(T2) then set_last_fix(T1<T2); assert last_fix still T2.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented in whereamid/src/db.rs.
- set_last_fix: added trailing "WHERE excluded.at_rfc3339 >= last_fix.at_rfc3339" to the ON CONFLICT DO UPDATE. SQLite upsert treats an unqualified/tablename-qualified column as the existing row and excluded.* as the proposed row; a false WHERE makes the conflict a no-op (no error).
- Doc-comment states the invariant verbatim: "monotonic by at_rfc3339 - older writes are silently dropped", plus the concurrency note (task-0046 serializes; this guards clock skew).
- insert_fix (AC#4): added a comment that the fixes timeseries is ordered by auto-increment id, NOT at_rfc3339, so no monotonicity guarantee; deliberately no CHECK (needs a migration and would reject legit skew observations).
GOTCHA for next engineer: the string ">=" only equals chronological order because every writer uses DateTime<Utc>::to_rfc3339() -> fixed-width prefix + constant "+00:00" offset. Verified AutoSi variable-width fractional seconds still sort correctly ("." 0x2E sorts after "+" 0x2B, so a no-fraction ts sorts before a same-second fractional one = chronologically correct). A non-UTC offset or a writer format change would silently break the guard.
SCOPE NOTE: kept the drop silent per AC. A dropped write is detectable via execute() returning 0 rows; a future task could debug-log skew events for observability.

Review-gate hardening (orchestrator): added the equal-timestamp regression assertion to set_last_fix_drops_older_timestamp, pinning the >= (not >) guard that the address-backfill path depends on. Commit c938b1c.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a DB-side timestamp monotonicity guard to set_last_fix so a wall-clock going backwards (NTP step, container time fix on restart) can no longer let a stale fix overwrite a newer one.

Changes (whereamid/src/db.rs):
- set_last_fix UPSERT now ends with WHERE excluded.at_rfc3339 >= last_fix.at_rfc3339; an older write becomes a no-op (SQLite upsert with a false WHERE does not error).
- Doc-comment names the invariant ("monotonic by at_rfc3339 - older writes are silently dropped") and documents why lexicographic string compare == chronological order for UTC to_rfc3339 output.
- insert_fix gains a comment (AC#4) clarifying the fixes timeseries is ordered by auto-increment id, not at_rfc3339, and intentionally has no monotonicity guard/CHECK.

Test: set_last_fix_drops_older_timestamp writes T2, then an older T1<T2 (dropped), then a newer T3 (applied) - asserting the >= guard both rejects older and accepts newer. Existing last_fix_round_trip (newer overwrite) still passes.

Gate: nix develop -c just e2e green (181 -> 183 aggregate; +1 test double-counted across lib+bin binaries), cargo fmt --check clean.
<!-- SECTION:FINAL_SUMMARY:END -->
