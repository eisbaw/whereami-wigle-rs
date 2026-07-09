---
id: TASK-0156
title: 'Test: Apple stray-BSSID filter (task-0049 regression)'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:30'
updated_date: '2026-07-09 04:20'
labels:
  - tests
  - apple
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P1-2. apple.rs:169 has 'if requested_set.contains(&ap.bssid)' filter (task-0049). No test feeds a response containing a BSSID NOT in requested and asserts it's dropped. Apple has historically responded with neighbours-of-requested BSSIDs in some configurations.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Hand-build a protobuf response with two WifiDevice entries; pass requested=[bssid_of_first] to decode_response; assert result has exactly 1 entry with the right BSSID
- [x] #2 Symmetric: requested=[both] -> result has 2 entries
- [x] #3 Symmetric: requested=[none-matching] -> result is empty
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Mirror push_wifi_device harness in apple.rs tests
2. Add symmetric-triple test: 2-entry response; requested=[first]->1, requested=[both]->2, requested=[]->empty
3. Assert unrequested BSSID dropped, requested kept
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added decode_response_drops_unrequested_bssids_symmetrically (apple.rs): one 2-entry response, asserted requested=[first]->1 (AC#1, stray SECOND dropped), [both]->2 (AC#2), [none]->empty (AC#3). BITES (reviewer + implementer: forcing the requested_set.contains filter off leaks the unrequested entry -> test fails). Complements the task-0148 3-entry golden (which never exercises the none-matching->empty path). Dual gate GO/GO (857).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Pinned the Apple stray-BSSID filter (task-0049): decode_response drops any returned BSSID not in the requested set. New test asserts all 3 cases (requested=first->1, both->2, none->empty) over one synthetic 2-entry response; bite-verified by forcing the filter off. Test-only; dual gate GO/GO; commit 344fbb9.
<!-- SECTION:FINAL_SUMMARY:END -->
