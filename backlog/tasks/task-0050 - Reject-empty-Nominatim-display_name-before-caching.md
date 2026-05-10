---
id: TASK-0050
title: Reject empty Nominatim display_name before caching
status: To Do
assignee: []
created_date: '2026-05-10 10:52'
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
- [ ] #1 reverse_geocode returns Err when display_name is missing or empty (not Ok with empty string)
- [ ] #2 Address cache never stores empty strings
- [ ] #3 Test for empty/missing display_name produces an Err and skips cache insert
<!-- AC:END -->
