---
id: TASK-0115
title: Extract handle_locate's address-backfill into nominatim.rs
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:13'
updated_date: '2026-07-09 06:59'
labels:
  - refactor
  - server
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:501-567. 60+ lines of nested mutex juggling (address_cache std → last_fix tokio → db std), one network call, two mutex-poison recoveries, plus a stale-fix check. Inline in the request handler. Densest critical-section reasoning in the codebase. task-0056 split out address_cache.rs but the handler itself wasn't sliced.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 New async fn backfill_address(state: Arc<DaemonState>, lat: f64, lon: f64) lives in nominatim.rs (or new src/address.rs)
- [x] #2 handle_locate becomes 'tokio::spawn(backfill_address(state.clone(), lat, lon))' — single line
- [x] #3 The lock-ordering story (last_fix before db, address_cache before either) lives inside the helper, testable in isolation
- [x] #4 Unit test: backfill_address with mocked Nominatim client (httpmock or similar); assert address_cache populated AND last_fix.address updated when stale
- [x] #5 Existing handle_locate tests still pass
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ALREADY SATISFIED — superseded by task-0193 (GeocodeScheduler), no code change needed. The inline 60-line address-backfill mutex-juggling that this task targeted (server.rs:501-567) was replaced by task-0193 with a superior design:
- AC#1 (extract the backfill to a module): the logic lives in server/geocode.rs (run_worker, ~314) + the reusable async helper backfill_last_fix_address_with (server.rs:257) — a dedicated geocode module, better than the AC-suggested nominatim.rs.
- AC#2 (handle_locate a single line): handle_locate now PUBLISHES to the scheduler via geocode.observe(); it never inline-juggles locks and never spawns a per-locate task (verified: server.rs:495-570 has no address_cache/last_fix/lock references). task-0193 is SUPERIOR to the AC\x27s tokio::spawn(backfill_address) — a one-worker latest-only scheduler coalesces bursts and avoids per-locate task spawns.
- AC#3 (lock-ordering inside the helper, testable in isolation): address_cache -> last_fix -> db ordering lives in backfill_last_fix_address_with + run_worker, in the geocode module with #[cfg(test)] tests.
- AC#4 (unit test, mocked Nominatim, asserts address_cache populated AND last_fix.address updated): geocode.rs tests use a mocked Nominatim transport (calls counter) and assert BOTH address_cache populated (line ~618) AND memory last_fix.address == Some("same-cell address") (line ~620), plus call-count == 1.
- AC#5 (existing handle_locate tests pass): green (part of the 890-test suite).
Closing as superseded-by-0193.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
No-op: the inline address-backfill mutex-juggling in handle_locate that this task asked to extract was already extracted by task-0193, which introduced a one-worker latest-only GeocodeScheduler (server/geocode.rs) + a reusable backfill_last_fix_address_with helper (server.rs). handle_locate now publishes to the scheduler in a single call and never inline-juggles locks or spawns a per-locate task — a superior realization of this task\x27s intent (the AC suggested tokio::spawn(backfill_address); the scheduler coalesces bursts instead). The lock-ordering lives in the helper, tested in isolation with a mocked Nominatim that asserts both address_cache population and last_fix.address backfill. Verified and closed as already-satisfied. No code change.
<!-- SECTION:FINAL_SUMMARY:END -->
