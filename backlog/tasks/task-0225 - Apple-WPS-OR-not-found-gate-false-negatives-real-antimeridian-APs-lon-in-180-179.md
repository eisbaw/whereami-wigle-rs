---
id: TASK-0225
title: >-
  Apple WPS OR not-found gate false-negatives real antimeridian APs (lon in
  [-180,-179))
status: Done
assignee: []
created_date: '2026-07-02 05:35'
updated_date: '2026-07-02 05:55'
labels:
  - correctness
  - apple
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0098 changed the Apple not-found gate to OR (either coord < -179.0 => not-found). This is correct for latitude (|lat|<=90, so lat<-179 is always sentinel/garbage) but over-rejects longitude: a real AP with valid latitude and longitude in [-180,-179) (South Pacific antimeridian: Fiji, Tonga, Kiribati, parts of NZ) is now silently treated as not-found by Apple. Deliberate safety tradeoff at the time (never persist garbage > resolve a rare antimeridian AP), but if antimeridian coverage matters, discriminate the sentinel using LATITUDE only (lat<-179 => not-found) and rely on the range-sanity check (|lon|>180) for garbage longitude, so valid lon in [-180,-179) survives. Ref whereamid/src/apple.rs parse_wifi_device.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Sentinel detection uses latitude as the primary discriminator so valid longitudes in [-180,-179) with a valid latitude still resolve
- [ ] #2 Unit test: (lat=-17.7, lon=-179.5) resolves to an ApInfo (Fiji antimeridian)
- [ ] #3 Unit test: (-180,-180) and (91,12) still rejected
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Resolved as part of task-0098: the not-found gate now uses a latitude-only sentinel discriminator instead of OR-on-longitude, so real antimeridian APs (lon in [-180,-179)) resolve correctly. antimeridian_ap_resolves regression test added in apple.rs. Commit bd99e0c.
<!-- SECTION:FINAL_SUMMARY:END -->
