---
id: TASK-0115
title: Extract handle_locate's address-backfill into nominatim.rs
status: To Do
assignee: []
created_date: '2026-05-20 21:13'
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
- [ ] #1 New async fn backfill_address(state: Arc<DaemonState>, lat: f64, lon: f64) lives in nominatim.rs (or new src/address.rs)
- [ ] #2 handle_locate becomes 'tokio::spawn(backfill_address(state.clone(), lat, lon))' — single line
- [ ] #3 The lock-ordering story (last_fix before db, address_cache before either) lives inside the helper, testable in isolation
- [ ] #4 Unit test: backfill_address with mocked Nominatim client (httpmock or similar); assert address_cache populated AND last_fix.address updated when stale
- [ ] #5 Existing handle_locate tests still pass
<!-- AC:END -->
