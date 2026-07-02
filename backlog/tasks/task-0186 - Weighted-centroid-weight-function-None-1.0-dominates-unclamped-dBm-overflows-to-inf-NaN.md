---
id: TASK-0186
title: >-
  Weighted-centroid weight function: None=>1.0 dominates; unclamped dBm
  overflows to inf/NaN
status: To Do
assignee: []
created_date: '2026-07-02 04:50'
labels:
  - correctness
  - math
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
trilaterate weight = match signal_dbm { Some(dbm) => 10f64.powf(dbm/20.0), None => 1.0 }. None=1.0 equals a 0 dBm signal: 100x the strongest realistic AP, 10000x a weak one, so one signal-less AP owns the centroid. dbm is never range-checked: powf overflows to +inf for large dbm (iw parser saturating cast can produce huge values), and inf/inf normalization yields NaN lat/lon returned as a successful Position. Dormant today (server always fills signal_dbm) but trilaterate is pub and fuzz/proptest exercise the None path. Ref whereamid/src/trilaterate.rs:416-426,470-477.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 a None-signal AP never receives more weight than the strongest measured AP in the same set (e.g. median weight or a nominal -75 dBm)
- [ ] #2 trilaterate returns finite lat/lon/accuracy for any i32 signal_dbm (clamp dbm to a physical window)
- [ ] #3 one named vote_weight helper is used by both trilaterate and estimate_accuracy
- [ ] #4 proptest: mixed Some/None inputs keep the centroid within the cluster diameter bound
<!-- AC:END -->
