---
id: TASK-0106
title: 'address_cache_key: hard-guard against NaN/Inf, not debug-only assert'
status: To Do
assignee: []
created_date: '2026-05-20 21:11'
labels:
  - robustness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server/address_cache.rs:32-39. address_cache_key quantizes to 1e-4° via 'as i32', then debug_assert\!s finiteness. debug_assert is compiled out in release. Today trilaterate doesn't produce NaN, but the property isn't statically enforced anywhere; a future code change could feed NaN through and the cache would silently insert under (0,0).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 AddressCache::get and AddressCache::insert return early (None / no-op) when lat or lon is not finite
- [ ] #2 Replace debug_assert\! with a real assertion at the function boundary, OR an Option<(i32,i32)> return for the key helper
- [ ] #3 Unit test: get(NaN, NaN) returns None and never mutates cache state; insert(NaN, ...) is a no-op
<!-- AC:END -->
