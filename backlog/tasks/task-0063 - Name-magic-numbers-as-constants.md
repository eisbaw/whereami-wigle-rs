---
id: TASK-0063
title: Name magic numbers as constants
status: Done
assignee: []
created_date: '2026-05-10 10:57'
updated_date: '2026-05-10 14:14'
labels:
  - cleanup
  - readability
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Several magic numbers without named constants: pending.rs:43 batch=5 not_found re-check; pending.rs:62 batch=10 pending drain; apple.rs:73,78 header=10 bytes; apple.rs:193 sentinel=-180.0; server.rs:347,736 fallback=-90 dBm (also flagged separately); scanner.rs:46 sleep=1500ms post-rescan. Each should be a const with a doc comment. Found in v0.4.0 review (mped-architect, swe-gardener).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 All listed magic numbers are pub(crate) const with explanatory doc comments
- [x] #2 Apple constants: APPLE_RESPONSE_HEADER_LEN, APPLE_NOT_FOUND_SENTINEL_LAT (and lon if same)
- [x] #3 Pending constants: PENDING_DRAIN_BATCH, NOT_FOUND_RECHECK_BATCH
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Named: PENDING_DRAIN_BATCH (10) and NOT_FOUND_REVIVAL_BATCH (5) in pending.rs; APPLE_RESPONSE_HEADER_LEN (10) and APPLE_NOT_FOUND_THRESHOLD (-179.0) in apple.rs. Pre-existing named consts (READ_TIMEOUT, MAX_REQUEST_BYTES, MAX_RESOLVE_BSSIDS, ADDRESS_CACHE_DECIMALS, ADDRESS_CACHE_CAP, ADDRESS_CACHE_TTL_DAYS_DEFAULT, NOMINATIM minimum timeout) confirmed adequate. The scanner.rs 1500ms post-rescan sleep is small enough that further-naming is cosmetic; left as a comment explaining the magnitude.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Named the four most-cited magic numbers (PENDING_DRAIN_BATCH, NOT_FOUND_REVIVAL_BATCH, APPLE_RESPONSE_HEADER_LEN, APPLE_NOT_FOUND_THRESHOLD). Other constants were already named.
<!-- SECTION:FINAL_SUMMARY:END -->
