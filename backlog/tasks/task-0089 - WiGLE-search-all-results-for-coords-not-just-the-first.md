---
id: TASK-0089
title: 'WiGLE: search all results for coords, not just the first'
status: To Do
assignee: []
created_date: '2026-05-13 21:54'
labels:
  - correctness
  - wigle
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
wigle.rs::lookup_bssid returns NotFound if the FIRST result lacks trilat/trilong: 'r.trilat.ok_or(WigleError::NotFound)?'. But WiGLE can return multiple records per BSSID (different sightings at different times, varying observer accuracy). A duplicate record from a sparse-area observation might be first in the list while a later record has coords. The current behavior poisons the BSSID for not_found_ttl_days (default 30 d, or 90 d depending on config) even though WiGLE actually has coords. We also consumed the upstream quota slot for nothing. Fix: iterate body.results and pick the first record where BOTH trilat and trilong are Some. If no result has coords, then it is a true NotFound. Two-line change.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 lookup_bssid scans body.results for the first record with both trilat and trilong Some
- [ ] #2 If no result has coords, returns WigleError::NotFound as today
- [ ] #3 If the first result lacks coords but a later one has them, the daemon resolves the AP and caches it (no false NotFound)
- [ ] #4 Test: WiGLE response with [{trilat:null,trilong:null}, {trilat:55.7,trilong:12.5}] resolves to (55.7,12.5)
- [ ] #5 Test: WiGLE response with all-null coords still returns NotFound
<!-- AC:END -->
