---
id: TASK-0272
title: >-
  Structural mobile-AP detection: locally-observed vs provider-reported
  disagreement
status: To Do
assignee: []
created_date: '2026-07-09 03:05'
labels:
  - behavioral
  - schema
  - mobile
dependencies:
  - TASK-0125
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The deep/structural half of task-0125 (split out so the safe ApInfo lat/lon rename+doc can land separately). Today is_mobile is SSID-denylist-based only (task-0093/0094 heuristic). This task adds the STRUCTURAL detector: persist each AP's locally-observed position (trilateration-consensus centroid) alongside the provider-reported centroid, and when they disagree by >1km repeatedly over N observations, mark is_mobile=1 and exclude from trilateration. This catches mobile APs (trains/buses/phones) that are NOT on the SSID denylist. Depends on task-0125 (the provider_reported_lat/lon rename that names the two concepts). Deep: schema migration v9->v10 (locally_observed_lat/lon NULL columns) + a new write path populating them from consensus + the N-observation disagreement algorithm + must coexist with (not double-count) the existing SSID-denylist is_mobile path. WANTS A FRESH CONTEXT + full dual gate.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Schema migration v9->v10 adds aps.locally_observed_lat/lon (NULL default), populated from trilateration consensus
- [ ] #2 Detection: when a BSSID's local-observation centroid disagrees with provider-reported by >1km repeatedly over N observations, aps.is_mobile is set 1 and the AP is excluded from trilateration
- [ ] #3 Coexists with the existing SSID-denylist is_mobile path without double-counting or regressing task-0094 behavior
- [ ] #4 Test: a train AP cached at provider-reported (55.74) but locally observed at (55.67) gets is_mobile=1 after N observations
<!-- AC:END -->
