---
id: TASK-0098
title: Apple WPS not-found gate uses && instead of ||
status: Done
assignee:
  - '@claude'
created_date: '2026-05-20 21:10'
updated_date: '2026-07-02 05:55'
labels:
  - correctness
  - apple
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/apple.rs:215-241. Sentinel for 'no position' is (-180, -180). Current check: 'if lat_f < -179.0 && lon_f < -179.0'. If Apple ever returns (-179.99, +12.34) — one coord sentinel-ish, other real — the AP is persisted as (-179.99, +12.34) under Source::Apple (top priority). Becomes sticky and uncorrectable by WiGLE without manual override. Apple's actual protocol uses both fields as sentinel together, but the conjunction makes the boundary fragile.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Add range sanity: reject |lat| > 90 or |lon| > 180 as protocol garbage (return None rather than persist)
- [x] #2 Unit test: response with (-179.99, 12.34) treated as not-found, no aps row written
- [x] #3 Unit test: response with (lat=91, lon=12) treated as malformed, no aps row written
- [x] #4 Not-found sentinel gate uses a latitude-only discriminator (lat < -179) backed by the range-sanity reject, so partial-sentinel/garbage is dropped WITHOUT false-negating valid antimeridian longitudes in [-180,-179)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. In parse_wifi_device gate: change lat_f/lon_f not-found conjunction from && to ||
2. Add range-sanity reject after the sentinel gate: |lat|>90 or |lon|>180 => return Ok(None)
3. Update the constant/comment to reflect OR semantics
4. Add unit tests: (-179.99,12.34)=>None (not-found), (91,12)=>None (malformed), plus a valid-coord control that still yields ApInfo
5. Run nix develop -c just e2e and just fmt
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented in parse_wifi_device (whereamid/src/apple.rs):
- not-found gate changed from && to || (either coord < -179.0 => not-found).
- added range-sanity reject: |lat|>90 or |lon|>180 => Ok(None) (never persisted).
- named MAX_ABS_LATITUDE/MAX_ABS_LONGITUDE constants (no magic numbers).

Tests (hand-encoded protobuf via encode_location_msg/encode_wifi_device_msg helpers):
- one_sentinelish_coordinate_is_not_found: (-179.99,12.34) => None (AC#3)
- out_of_range_latitude_is_malformed: (91,12) => None (AC#4)
- valid_coordinate_still_resolves: (55.68,12.57) => Some (over-aggression control)
- double_sentinel_still_not_found: (-180,-180) => None (regression guard)

LIMITATION (self-critical): the OR on longitude adds a rare FALSE NEGATIVE. A real AP with a valid latitude but longitude in [-180,-179) (South Pacific antimeridian: Fiji/Tonga/Kiribati) now reads as not-found. This is the deliberate safety tradeoff per the task theme (never persist garbage under top-priority Source::Apple > resolving a rare antimeridian AP; WiGLE can still resolve it, or it stays pending). Filed follow-up task to revisit with a latitude-only sentinel discriminator if antimeridian coverage ever matters.

Gate: nix develop -c just e2e + just fmt both green. Tests 185 -> 193 (4 new x2, lib+bin double-count). 0 clippy warnings, fmt clean.

Forward-carried to task-0187: the sentinel/range gate lives in parse_wifi_device; 0187 changes parse_location to return Options. double_sentinel_still_not_found already guards 0187 AC#3 (existing -180 behavior unchanged).

Review-gate correction (orchestrator): the original AC#1 "change && to ||" was a defective prescription — the || overcorrected and REGRESSED real South-Pacific antimeridian APs (valid lat, lon in [-180,-179), e.g. Fiji lon=-179.9). The range-sanity check (AC#2) is the actual fix for the (-179.99,12.34) motivating case (|lat|>90). Reimplemented as a latitude-only sentinel gate + range reject (strictly better; all prior tests pass unchanged) and added antimeridian_ap_resolves regression test. Commit bd99e0c. AC#1 reworded to the correct outcome. This also closes task-0225.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Apple WPS not-found gate hardened: latitude-only sentinel discriminator (lat < -179) plus range-sanity reject (|lat|>90 or |lon|>180 => drop, never persist under top-priority Source::Apple). Rejects partial-sentinel/garbage like (-179.99,12.34) and out-of-range (91,12) while correctly resolving valid antimeridian APs. 5 unit tests (incl antimeridian regression guard). Commits 6abcd98 + bd99e0c.
<!-- SECTION:FINAL_SUMMARY:END -->
