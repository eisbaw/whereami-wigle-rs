---
id: TASK-0187
title: >-
  Apple parse_location defaults missing lat/lon to 0 -> fabricates Null Island
  AP
status: Done
assignee:
  - '@claude'
created_date: '2026-07-02 04:50'
updated_date: '2026-07-02 05:38'
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
- [x] #1 a WifiDevice whose Location has no field-1/field-2 produces no ApInfo (track lat/lon as Option, skip device when either absent)
- [x] #2 unit test with a hand-encoded empty Location message pins this
- [x] #3 existing -180 sentinel behavior unchanged
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Change parse_location to return (Option<i64>, Option<i64>); init lat/lon as None, set Some(val) only when field 1/2 present.
2. In parse_wifi_device, assign lat=lt; lon=ln directly so a missing coord stays None and the existing (Some,Some,Some) match arm skips the device.
3. Update doc comments to reflect optional semantics.
4. Tests: empty Location => no ApInfo; parse_location(&[]) => (None,None); Location with only latitude => no ApInfo. AC#3 (-180 sentinel) already guarded by double_sentinel_still_not_found from 0098.
5. Gate: nix develop -c just e2e + just fmt.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented in whereamid/src/apple.rs:
- parse_location now returns (Option<i64>, Option<i64>); lat/lon init None, set Some only when field 1/2 is present. Absent field => None instead of 0.
- parse_wifi_device assigns lat=lt; lon=ln directly; the existing (Some,Some,Some) match arm skips the device when either coord is None. An empty/partial Location can no longer become a (0,0) Null Island fix under Source::Apple.

Tests:
- parse_location_empty_is_none_none: parse_location(&[]) == (None,None)
- empty_location_produces_no_apinfo: WifiDevice with empty Location => None (AC#1/#2)
- location_with_only_latitude_produces_no_apinfo: only field 1 present => None (AC#1)
- AC#3 (-180 sentinel unchanged) guarded by double_sentinel_still_not_found (added in 0098), still green.

Callers checked: only parse_wifi_device (updated) and fuzz_apple_decode.rs (uses `let _ =`, unaffected; fuzz crate is not a workspace member). No other callers.

Gate: nix develop -c just e2e + just fmt green. Tests 193 -> 199 (3 new x2, lib+bin double-count). 0 clippy warnings, fmt clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Stop Apple WPS from fabricating a (0,0) Null Island AP from an empty/partial Location (task-0187).

What changed:
- whereamid/src/apple.rs parse_location: return type (i64,i64) -> (Option<i64>,Option<i64>). latitude (field 1) and longitude (field 2) are optional int64 in AppleWLoc.proto; an absent field is now None instead of defaulting to 0.
- parse_wifi_device propagates those Options directly, so the existing (Some,Some,Some) match arm skips any device whose Location is empty or carries only one coordinate.

Why: a truncated/format-drifted response with an empty Location previously yielded (0,0) -> lat=Some(0) -> passed the < -179 sentinel check -> ApInfo at Null Island cached under top-priority Source::Apple. Part of the "never persist garbage" theme.

Tests: added 3 unit tests (empty Location => None, parse_location empty => (None,None), latitude-only => None). The -180 sentinel regression is covered by double_sentinel_still_not_found. Workspace 193 -> 199 passing. clippy -D warnings clean, cargo fmt --check clean.

Risk/scope: the only non-test caller is parse_wifi_device; the fuzz target discards the return value and the fuzz crate is not a workspace member, so no build breakage.
<!-- SECTION:FINAL_SUMMARY:END -->
