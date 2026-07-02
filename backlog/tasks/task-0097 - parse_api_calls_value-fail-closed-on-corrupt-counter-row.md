---
id: TASK-0097
title: 'parse_api_calls_value: fail-closed on corrupt counter row'
status: Done
assignee:
  - '@claude'
created_date: '2026-05-20 21:10'
updated_date: '2026-07-02 05:11'
labels:
  - correctness
  - wigle
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/db.rs:52-66. parse_api_calls_value returns 0 when metadata.api_calls_today is non-numeric. Combined with the daemon's optimistic mirror of WiGLE's server-side counter, a corrupted row (disk gremlins, partial write) gets healed by the next try_reserve_api_call writing the bumped value over the garbage. Daemon then issues up to daily_limit fresh WiGLE calls today on top of whatever was already spent — silently double-spending an irreversible API budget. task-0077 added a warn; consequence is not contained.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 On parse failure, parse_api_calls_value (or its caller try_reserve_api_call) returns Err / treats the count as daily_limit (deny-all) rather than 0 (allow-all)
- [x] #2 Provider treats the Err as ProviderOutcome::Skipped (no quota spent, no row corruption)
- [x] #3 Unit test: corrupt metadata.api_calls_today to 'garbage', call try_reserve_api_call, assert it returns false and the row is not overwritten
- [x] #4 tracing::warn fires once per corruption (already exists from task-0077); now paired with fail-closed behavior
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. parse_api_calls_value -> Result<u32>: Ok(0) for None, Ok(v) valid, Err(+warn) corrupt.
2. try_reserve_api_call: on parse Err, early-return Ok(false) (deny); tx rolls back so corrupt row not overwritten.
3. refund_api_call + api_calls_today: propagate Err via ?; api_calls_today caller already unwrap_or(0).
4. Update warn message to reflect deny.
5. Unit test: corrupt row -> try_reserve Ok(false), row unchanged.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented in whereamid/src/db.rs.
- parse_api_calls_value now returns Result<u32>: Ok(0) for absent key, Ok(v) for valid, Err(+warn) for a present-but-non-numeric value.
- try_reserve_api_call: on parse Err it early-returns Ok(false) (deny). rusqlite Transaction rolls back on drop, so no write occurs and the corrupt row is left untouched. Chose Ok(false) (deny-all) over propagating Err to satisfy AC#3 literal "returns false"; provider maps BOTH Ok(false) and Err to Skipped, so AC#2 outcome (Skipped, no spend, no corruption) holds either way.
- refund_api_call also propagates Err now: previously it would heal garbage to "0" (a mini version of the same double-spend heal). Required by the signature change and is the correct containment.
- api_calls_today returns Err on corruption; its only caller (server.rs:986 status display) uses .unwrap_or(0), so stats render 0 without affecting enforcement.
- AC#2 verified against existing provider code (provider.rs wigle_lookup): no new provider test needed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Fail closed on a corrupt WiGLE daily-counter row so a garbage value can no longer reset the counter and re-charge up to daily_limit fresh API calls on top of what was already spent.

Changes (whereamid/src/db.rs):
- parse_api_calls_value: Option<String> -> Result<u32>. None => Ok(0); non-numeric => Err with a warn (task-0077 warn retained, message now describes the deny action).
- try_reserve_api_call: corrupt counter => early return Ok(false) (deny) without writing; the uncommitted transaction rolls back, leaving the row untouched.
- refund_api_call + api_calls_today: propagate the Err (refund no longer heals garbage to 0; status display uses unwrap_or(0)).
- Provider already maps a denied/errored reservation to ProviderOutcome::Skipped (no quota spent), satisfying AC#2 with no provider change.

Test: try_reserve_fails_closed_on_corrupt_counter seeds api_calls_today='garbage' for today, asserts try_reserve_api_call returns false and the row is still 'garbage'.

Gate: nix develop -c just e2e green (179 -> 181 aggregate; +1 test double-counted across the lib+bin test binaries), cargo fmt --check clean.
<!-- SECTION:FINAL_SUMMARY:END -->
