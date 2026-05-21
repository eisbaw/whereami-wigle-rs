---
id: TASK-0129
title: 'Stay-point detection: first-fix anchor + accuracy-aware threshold'
status: To Do
assignee: []
created_date: '2026-05-20 21:20'
labels:
  - correctness
  - history
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #8. history.rs:103-110 segment_fixes computes d=haversine_m(centroid, new_fix) where centroid is the geometric_median of accepted fixes so far. As you accept more fixes the centroid shifts — a fix 95m from centroid but 180m from the FIRST fix in the segment is still accepted with dist_threshold_m=100. Over a long drift, a stay-point can stretch hundreds of meters. Also: default 100m threshold is smaller than the trilateration accuracy admitted (clamp 10..1000). Two fixes at same physical stop with 800m accuracy each get split into different segments. The thresholds don't take per-fix accuracy into account.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Stay-point membership: haversine_m(first_fix_in_segment, new_fix) <= dist_threshold_m (not running-centroid)
- [ ] #2 OR: incorporate accuracy: haversine_m(...) - (acc_i + acc_j) <= dist_threshold_m so wide-accuracy fixes merge correctly
- [ ] #3 Add max_gap_secs (default 3600s): if (new_fix.at - last_accepted.at) > max_gap_secs, force a new segment (currently a stale gap can append to a segment hours later)
- [ ] #4 Rename CLI flags: --stay-point-radius-m (was --history-segment-distance-m), --stay-point-min-dwell-secs (was --history-segment-min-duration-secs), --stay-point-max-gap-secs (new); old names accepted as aliases for one release
- [ ] #5 Existing history tests update accordingly; new test for the first-fix anchor behavior and the max-gap behavior
<!-- AC:END -->
