---
id: TASK-0124
title: Rename trilaterate -> weighted_centroid (honest naming)
status: To Do
assignee: []
created_date: '2026-05-20 21:18'
labels:
  - docs
  - domain
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #3. 'Trilateration' uses distances from 3+ known anchors. 'Multilateration' uses TDoA. Neither is what this code does — it computes a signal-strength-weighted spherical centroid. No path-loss inversion, no distance estimate, no intersection of spheres. The function comment at trilaterate.rs:419 correctly says 'Weighted spherical mean'; the function name disagrees. Users reading the code will assume an accuracy floor we don't have.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Rename pub fn trilaterate to pub fn weighted_centroid (or estimate_position_from_aps)
- [ ] #2 Keep a thin alias 'pub use weighted_centroid as trilaterate;' for one release for external callers
- [ ] #3 Rename trilaterate.rs to centroid.rs (or keep, your call — document in task notes)
- [ ] #4 Doc-comment explicitly says: 'NOT trilateration (no calibrated path-loss model). Weighted spherical centroid of cached AP positions. RSSI provides voting weight, not distance.'
- [ ] #5 README + docs/protocol.md + CHANGELOG updated to match
- [ ] #6 Existing tests pass; the wire-format-visible name 'trilaterate' (if any) preserved or migrated cleanly
<!-- AC:END -->
