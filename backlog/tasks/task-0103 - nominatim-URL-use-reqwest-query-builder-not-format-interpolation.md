---
id: TASK-0103
title: 'nominatim URL: use reqwest query builder, not format\!() interpolation'
status: To Do
assignee: []
created_date: '2026-05-20 21:11'
labels:
  - robustness
  - nominatim
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/nominatim.rs:76-78. URL constructed via format\!() with unescaped lat/lon. For trilaterate-derived floats this is fine. But the function is pub and reachable from any future 'user pastes in coordinates' feature. A NaN/Inf would produce a malformed URL; a manually-crafted lat like '0&radius=1000' would be query-string injection.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Use Client::get(base).query(&[("lat", lat), ("lon", lon), ...]) so reqwest handles escaping
- [ ] #2 Unit test: reverse-geocode call with f64::NAN returns Err (not a request to the network)
- [ ] #3 Doc-comment: 'lat/lon must be finite; caller is responsible'
<!-- AC:END -->
