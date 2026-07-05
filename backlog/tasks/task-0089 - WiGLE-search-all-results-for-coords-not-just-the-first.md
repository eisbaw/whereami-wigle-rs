---
id: TASK-0089
title: 'WiGLE: search all results for coords, not just the first'
status: Done
assignee:
  - task0089-implementer
created_date: '2026-05-13 21:54'
updated_date: '2026-07-05 11:07'
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
- [x] #1 lookup_bssid scans body.results for the first record with both trilat and trilong Some
- [x] #2 If no result has coords, returns WigleError::NotFound as today
- [x] #3 If the first result lacks coords but a later one has them, the daemon resolves the AP and caches it (no false NotFound)
- [x] #4 Test: WiGLE response with [{trilat:null,trilong:null}, {trilat:55.7,trilong:12.5}] resolves to (55.7,12.5)
- [x] #5 Test: WiGLE response with all-null coords still returns NotFound
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Make the actual results array authoritative and choose the first record containing both latitude and longitude.
2. Preserve NotFound only when no returned record has a complete coordinate pair; copy all metadata from the selected record.
3. Add table/fixture coverage for a sparse first record followed by a positioned record, all-null records, and no records; rely on existing resolver write-through test for cache composition unless a gap remains.
4. Run focused tests, mandatory parallel reviews, final E2E, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation: interpret_response now treats the actual results array as authoritative, selects the first response-ordered record containing both trilat and trilong, and copies coordinates plus all metadata from that same record. Empty, all-null, and one-sided-only arrays remain NotFound. The unused totalResults field was removed, so an inconsistent count cannot suppress a positioned record.

Tests pin sparse-first/later-complete (55.7,12.5), first-complete ordering among multiple positioned records, metadata consistency, one-sided skipping, all-incomplete NotFound, and empty NotFound. Existing resolver Found-to-cache coverage establishes write-through composition. Frozen hash 22408e299e3a4d2adfe903a30a801e25ee5514183afb478112deb8cbf19ba23b received QA GO and architecture GO; 514 E2E tests and strict clippy passed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
WiGLE responses now scan all returned records and use the first complete coordinate pair, preventing false 30-day negative-cache entries when only the first record is sparse. Inconsistent totalResults counts no longer override actual positioned data, and metadata stays tied to the selected record.
<!-- SECTION:FINAL_SUMMARY:END -->
