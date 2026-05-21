---
id: TASK-0098
title: Apple WPS not-found gate uses && instead of ||
status: To Do
assignee: []
created_date: '2026-05-20 21:10'
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
- [ ] #1 Change && to || at the not-found gate
- [ ] #2 Add range sanity: reject |lat| > 90 or |lon| > 180 as protocol garbage (return None rather than persist)
- [ ] #3 Unit test: response with (-179.99, 12.34) treated as not-found, no aps row written
- [ ] #4 Unit test: response with (lat=91, lon=12) treated as malformed, no aps row written
<!-- AC:END -->
