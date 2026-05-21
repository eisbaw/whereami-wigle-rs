---
id: TASK-0153
title: 'Test: trilaterate proptest exercises full production path (no pre-filter)'
status: To Do
assignee: []
created_date: '2026-05-20 21:30'
labels:
  - tests
  - proptest
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P0-6. tests/proptests.rs:38 trilaterate_within_cluster_diameter pre-applies filter_outliers before calling trilaterate. But trilaterate already runs filter_outliers internally (trilaterate.rs:399). The property is correct for the inner math; no proptest feeds RAW outlier-laden input to the public API. A refactor disabling the inner filter_outliers call would still pass.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Drop the manual filter_outliers precall in trilaterate_within_cluster_diameter
- [ ] #2 Inject random outlier in addition to the cluster (e.g. one AP placed at lat ± 5° from the cluster centroid)
- [ ] #3 Assert the centroid stays within the CLUSTER's diameter (not the diameter including the outlier)
- [ ] #4 Test fails if someone removes the inner filter_outliers call (verify by temporarily commenting it out)
<!-- AC:END -->
