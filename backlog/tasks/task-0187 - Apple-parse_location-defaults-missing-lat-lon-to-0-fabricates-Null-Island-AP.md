---
id: TASK-0187
title: >-
  Apple parse_location defaults missing lat/lon to 0 -> fabricates Null Island
  AP
status: To Do
assignee: []
created_date: '2026-07-02 04:50'
labels:
  - correctness
  - math
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
parse_location initialises lat=0,lon=0; Location.latitude/longitude are optional int64 in AppleWLoc.proto. A WifiDevice whose Location submessage is empty or carries only skipped fields yields (0,0) -> lat=Some(0) -> passes the not-found sentinel check (< -179) -> ApInfo{lat:0,lon:0} at Null Island enters the cache with Source::Apple. Edge-reachable on format drift or truncated/malformed responses; the resulting confident-but-wrong cache row survives outlier filtering next to a poisoned sibling. Ref whereamid/src/apple.rs:245-268, consumed 203-239.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 a WifiDevice whose Location has no field-1/field-2 produces no ApInfo (track lat/lon as Option, skip device when either absent)
- [ ] #2 unit test with a hand-encoded empty Location message pins this
- [ ] #3 existing -180 sentinel behavior unchanged
<!-- AC:END -->
