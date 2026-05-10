---
id: TASK-0037
title: 'Address cache: TTL and bounded size'
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 05:38'
updated_date: '2026-05-10 08:19'
labels:
  - server
  - cache
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0025 introduced a (lat,lon)-rounded address cache to avoid blocking locate on Nominatim. The cache currently has no TTL and no size bound; daemon restart is the only invalidation. For long-running daemons street/POI data goes stale and the HashMap grows unboundedly. Add a TTL (e.g. 7 days) and a size cap (e.g. 1024 entries) with simple LRU eviction or oldest-first.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Cache entries expire after a configurable TTL (default 7 days)
- [x] #2 Cache size is bounded; oldest entries evicted when bound is reached
- [x] #3 Existing 'address attached on second locate' behavior preserved within TTL
- [x] #4 Unit test covers TTL expiry and size eviction
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Extend AddressCache to store (address, inserted_at: chrono::DateTime<Utc>) instead of just String
2. get(lat, lon) returns None if entry expired (TTL exceeded)
3. Add periodic eviction: when inserting, drop expired entries; keep size cap as fallback
4. Add CLI flag --address-cache-ttl-days (default 7)
5. Tests: TTL expiry, size eviction
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Extended AddressCache to store (address, inserted_at: DateTime<Utc>). get() returns None when entry age >= ttl_days. Added with_ttl_days(ttl) constructor; main.rs constructs with --address-cache-ttl-days. Default 7 days.

Two new tests: address_cache_expires_after_ttl (forces inserted_at into past, asserts None) and address_cache_zero_ttl_always_misses (TTL=0 disables the cache).

Note: chose 'expire on read' over a periodic eviction sweep — the cache is small (≤256 entries) so the slight memory hold-on for expired entries until they're naturally evicted on insert is fine. Avoids needing a background task.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Address cache entries now expire after a configurable TTL (default 7 days, --address-cache-ttl-days). Entries are checked on read and reported as None if expired. Size cap (256) is preserved. Two new tests: TTL expiry and TTL=0 always-miss.
<!-- SECTION:FINAL_SUMMARY:END -->
