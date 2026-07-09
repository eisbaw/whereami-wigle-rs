---
id: TASK-0160
title: 'Test: try_reserve_api_call date-rollover (timezone safety)'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:31'
updated_date: '2026-07-09 04:39'
labels:
  - tests
  - wigle
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P1-6. db.rs:665. The reset logic at lines 670-693 runs when stored_date \!= today. Test at :1241 never advances the clock; the rollover path is dead-as-tested. Replacing Utc::now().format('%Y-%m-%d') with local_now() would silently reset counters at random local times for users in negative-UTC timezones.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Test: manually insert metadata.api_calls_date='1999-01-01', api_calls_today='99'
- [x] #2 Call try_reserve_api_call(100); assert returned true, api_calls_today == 1 after, api_calls_date is today (UTC)
- [x] #3 Test that an explicit UTC date is used regardless of TZ env var (set TZ=America/Los_Angeles inside the test for the duration if portable)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add sync unit test in db.rs tests mod near other rate-limit tests
2. Seed api_calls_date=2020-01-01 + wigle_lookups_today=99 via set_metadata; call try_reserve_wigle_lookup(5); assert Reserved, counter==1, date==today(UTC)
3. Negative pin: seed api_calls_date=today + counter at limit; assert Exhausted
4. Verify via nix develop -c just e2e
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Clock-free rollover proof in db.rs tests (commit 1fcff34):
- stale_date_resets_counter_and_grants_reservation: seed api_calls_date=2020-01-01 + wigle_lookups_today=99, limit=5 -> Reserved; after: api_calls_date=today(UTC), counter=1.
- same_day_over_limit_denies_reservation: api_calls_date=today + counter=limit -> Exhausted, counter untouched.
- BITE verified by mutation (force the reset branch off): stale test flips Reserved->Exhausted and fails; same_day still passes. Reverted.
- Placed as sync #[cfg(test)] unit tests near two_connections_cannot_overbook_one_quota_slot; uses set_metadata + Utc::now, no clock injection. AC#3 (TZ env var) NOT covered: code already hard-codes Utc::now, and mutating process-global TZ mid-suite is non-portable/racy under the parallel harness.

## Review gate (orchestrator, independent) — GO/GO
- Two clock-free db tests: stale_date_resets_counter_and_grants_reservation (seed api_calls_date=2020-01-01 + wigle_lookups_today=99, reserve(5) -> Reserved, counter reset to 1, date=today) + same_day_over_limit_denies_reservation (negative pin: today + at-limit -> Exhausted, counter untouched). Deterministic (raw set_metadata seeding, no clock injection). BITE confirmed: reviewer traced db.rs:3249/3275 — removing the date-mismatch reset flips the stale case Reserved->Exhausted.
- AC#3 (TZ-env independence) satisfied BY CONSTRUCTION, not via an env-var test: the reset keys off Utc::now() (db.rs:3232), which ignores TZ entirely, so a TZ-env test would exercise nothing and would be non-portable/racy under the parallel harness. Reviewer confirmed skipping the env test is correct, NOT a coverage gap.
- LOW (accepted, reviewer-endorsed): a microsecond-per-day midnight window exists because the test and the code each call Utc::now() separately; effectively never fires (a UTC-midnight tick must land in the microsecond gap). Documented as an accepted limitation; a clock seam would be a production change out of scope.
- Dual gate GO/GO (869 tests, zero flake across QA 2x + 5x targeted).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Pinned that the daily WiGLE-lookup quota counter resets on UTC date rollover. Two clock-free tests: a stale persisted api_calls_date with an over-limit counter grants the next reservation (reset fired) and re-stamps today\x27s UTC date + counter 1; a same-day at-limit counter denies (Exhausted). Deterministic via raw metadata seeding (no clock control); bite-verified by mutation. AC#3 (TZ independence) is satisfied by construction — the code uses Utc::now(), ignoring TZ — so an env-var test would be a no-op; skipping it is correct per review. Test-only; dual gate GO/GO; commit 1fcff34.
<!-- SECTION:FINAL_SUMMARY:END -->
