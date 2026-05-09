---
id: TASK-0022
title: Fix rate limit race condition
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-09 21:07'
updated_date: '2026-05-09 21:52'
labels: []
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Rate limit check (can_call_api) and record (record_api_call) are separated by an HTTP call. Multiple concurrent tasks can pass the check before any records the call, exceeding the daily limit. Use atomic counter or hold a separate Mutex/semaphore for API call gating.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 try_reserve_api_call performs read+increment in single SQL txn
- [x] #2 refund_api_call exists and clamps at 0
- [x] #3 Old can_call_api/record_api_call removed (no callers after refactor)
- [x] #4 Concurrent test demonstrates total reservations <= daily_limit under contention
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add Database::try_reserve_api_call(daily_limit) using a single SQL transaction (immediate) so the read-modify-write is atomic against any other Connection (no other writers exist yet, but we keep the property as a guarantee).
2. Add Database::refund_api_call() that decrements the counter (clamped at 0), inside a transaction.
3. Mark old can_call_api / record_api_call as deprecated — actually delete them since per CLAUDE.md no backwards-compat shims, and only one caller.
4. Update provider::wigle_lookup to: try_reserve_api_call -> on false return Skipped; do the await; on Network/RateLimited refund; on Found/NotFound burn.
5. Add a concurrent test: spawn N std::thread on a shared Database with daily_limit=10, each calling try_reserve_api_call. Need Arc<Mutex<Database>> wrapper because Connection is \!Send-shareable. Sum reserved == min(N, daily_limit).
6. Build, clippy, test.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Replaced can_call_api/record_api_call with single atomic try_reserve_api_call(daily_limit) that does daily reset + read + increment in one IMMEDIATE SQL transaction. Added refund_api_call() (clamped) for transient failures.

Provider now reserves before await; refunds on RateLimited/Network; consumes on Found/NotFound (matches old accounting).

Added test_rate_limiting_concurrent: 64 threads, limit=10 -> exactly 10 reservations.

Limitation: Database is currently held under an outer std::sync::Mutex, which already serializes access in this codebase. The SQL transaction is therefore belt-and-braces for a future move to a connection pool or per-thread connections. Decided to keep it because it is the natural way to express the atomic invariant and costs nothing.

build/clippy/fmt/test all clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Eliminate TOCTOU between WiGLE quota check and quota record.

Changes:
- db.rs: replace can_call_api/record_api_call with try_reserve_api_call(daily_limit) — daily-counter reset, read, and increment now happen in a single SQLite IMMEDIATE transaction. Add refund_api_call() to give a slot back on transient failures (clamped at 0; no-op if the calendar day rolled over).
- provider.rs::wigle_lookup: reserve a slot synchronously before the network call; on Found/NotFound the slot is consumed; on RateLimited/Network the slot is refunded so transient failures do not burn quota (matching the prior accounting behavior).
- Removed dead set_metadata helper.
- New unit test test_rate_limiting_concurrent: 64 threads racing on a shared Database with daily_limit=10 produce exactly 10 reservations.

Why: The old code released the DB mutex during the WiGLE HTTP call, so two concurrent lookups could each observe count<limit and each proceed, exceeding the daily quota by N. With reserve-then-call the slot is committed before any await point.

Limitation worth noting: the daemon already serializes DB access via an outer std::sync::Mutex<Database>, so the inner SQL transaction is currently redundant for inter-thread safety. It is kept as the locally meaningful atomic primitive so future refactors (connection pool, RwLock, multiple writers) cannot silently break the invariant.

Tests: cargo build, cargo clippy --all-targets -D warnings, cargo fmt --check, cargo test (25 unit + 9 proptest) all clean.
<!-- SECTION:FINAL_SUMMARY:END -->
