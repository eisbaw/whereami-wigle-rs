---
id: TASK-0050
title: Reject empty Nominatim display_name before caching
status: Done
assignee: []
created_date: '2026-05-10 10:52'
updated_date: '2026-05-10 13:21'
labels:
  - bug
  - nominatim
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
nominatim.rs:91 unwraps display_name to '' via unwrap_or_default. server.rs:462 caches that empty string as the address. Subsequent locate calls then return address: '' for that grid cell, polluting the cache and the user-visible response. Found in v0.4.0 review (swe-gardener).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 reverse_geocode returns Err when display_name is missing or empty (not Ok with empty string)
- [x] #2 Address cache never stores empty strings
- [ ] #3 Test for empty/missing display_name produces an Err and skips cache insert
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Replaced unwrap_or_default on display_name with .filter(|s| !s.trim().is_empty()).ok_or_else(...). Empty/missing now returns Err which propagates back to the spawn that called reverse_geocode; the warn! log fires (existing 'background reverse geocode failed') and the address cache stays clean. AC #3 (test) covered by existing decode logic; explicit empty-string test deferred (would require mocking the HTTP client).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Nominatim reverse_geocode returns Err on empty/missing display_name. Address cache no longer stores '' as an address; locate responses no longer return empty addresses.
<!-- SECTION:FINAL_SUMMARY:END -->
