---
id: TASK-0025
title: Nominatim should not block locate response
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-09 21:07'
updated_date: '2026-05-09 22:36'
labels: []
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
handle_locate calls Nominatim synchronously in the response path. Slow Nominatim (1-5s) blocks the locate response even though position was computed instantly. Nominatim rate limiter mutex also serializes concurrent locate requests. Fix: return position immediately, resolve address asynchronously and cache it.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a small bounded address cache keyed by rounded (lat, lon) at ~10m grid (4 decimals). Use a custom small HashMap-with-bounded-cap structure to avoid pulling in the lru crate for one feature.
2. Cache lives in DaemonState behind a std::sync::Mutex (no awaits).
3. handle_locate computes pos, then:
   - if address_approx and cache hit: attach address to response immediately and to last_fix.
   - if address_approx and cache miss: spawn background task that calls Nominatim, then writes into both cache AND last_fix.address.
   - if not address_approx: behaviour unchanged (no Nominatim, no cache touch).
4. Sanity: position computation latency dominated by trilateration only.
5. Build / test / clippy / fmt clean.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added a small bounded address cache (256 entries, 4-decimal grid keying ~11m at equator) on DaemonState behind std::sync::Mutex (no awaits held). handle_locate probes the cache synchronously; on miss it spawns a background tokio task that calls Nominatim, populates the cache, and back-fills last_fix.address if the position has not changed.

Limitations / honest gotchas:
- First locate at a new rounded position returns address=null; the next locate at the same grid cell hits.
- No TTL: a daemon restart is the only way to invalidate. Acceptable for single-user deployment, but worth noting if street/POI metadata becomes important.
- Background task does NOT update the response that triggered it; that response has already been serialised.
- The 1 req/s Nominatim mutex still serialises background tasks, but it no longer blocks the locate hot path.

Unit tests: address_cache_key rounding boundary; AddressCache get/insert/eviction-cap.
All cargo build / test / clippy / fmt clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Decoupled Nominatim reverse geocoding from the locate response path. Locate now returns at trilateration speed instead of being capped by Nominatim latency (~1s) and its 1 req/s rate-limit mutex.

Design:
- New AddressCache (whereamid/src/server.rs): bounded HashMap keyed by 4-decimal-rounded (lat, lon) ≈ 11m grid, soft cap 256 entries with insertion-order eviction. Tiny custom struct, no new deps.
- DaemonState gains address_cache: std::sync::Mutex<AddressCache> (no awaits held).
- handle_locate: cache hit -> attach address synchronously. Miss -> attach None, spawn background task that calls Nominatim and writes both into the cache and (if still applicable) last_fix.address.
- --address-approx flag still gates whether Nominatim is called at all; behavior with the flag off is unchanged.

Trade-offs (called out in task notes):
- First locate at a new grid cell returns address=null; the next call at the same cell hits.
- No TTL on the cache; daemon restart is the only invalidation. Acceptable for single-user deployment.

Tests: 2 new unit tests (key rounding, get/insert/eviction). All cargo build / test / clippy / fmt clean.
<!-- SECTION:FINAL_SUMMARY:END -->
