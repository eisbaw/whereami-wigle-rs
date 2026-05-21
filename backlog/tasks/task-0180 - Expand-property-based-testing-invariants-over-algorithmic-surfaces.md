---
id: TASK-0180
title: 'Expand property-based testing: invariants over algorithmic surfaces'
status: To Do
assignee: []
created_date: '2026-05-20 21:36'
labels:
  - tests
  - proptest
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Today's proptests (9): debounce window/threshold/forget, parser round-trips for nmcli, trilaterate cluster-diameter (currently flawed per task-0153), outlier_filter_never_empty. Coverage gaps are algorithmic invariants that the current sampled-tests don't pin.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add proptest: filter_outliers + trilaterate composed — result is always inside the cluster's bounding box for any random outlier configuration (after task-0153 fix is the cleaner version)
- [ ] #2 Add proptest: drop_isolated invariant — for any input, every survivor has at least one neighbor within NEIGHBOR_RADIUS_M (modulo the 'all isolated' fallback)
- [ ] #3 Add proptest: geometric_median bounding-box invariant (covered by task-0161 Weiszfeld convergence proptest — coordinate scope)
- [ ] #4 Add proptest: haversine_m symmetry haversine(a,b)==haversine(b,a) and triangle inequality
- [ ] #5 Add proptest: segment_fixes invariants — every segment's centroid is within dist_threshold of all its constituent fixes (modulo accuracy); no segment shorter than min_duration; segments are non-overlapping in time
- [ ] #6 Add proptest: try_reserve_api_call - random sequences of reserve/refund preserve the invariant 'count never exceeds daily_limit'
- [ ] #7 Add proptest: AddressCache LRU+TTL eviction invariant - oldest entries evicted first, TTL-expired entries never returned
- [ ] #8 Add proptest: Bssid newtype round-trip (after task-0122) — for any valid 6-byte MAC, parse -> Display -> parse is identity
- [ ] #9 All new proptests run in <30s combined; add to 'just test'
- [ ] #10 Each proptest pins a SUFFICIENT condition for correctness; a refactor that breaks the invariant must fail the proptest
<!-- AC:END -->
