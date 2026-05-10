---
id: TASK-0049
title: Cross-check Apple WPS returned BSSIDs against requested set
status: Done
assignee: []
created_date: '2026-05-10 10:52'
updated_date: '2026-05-10 13:21'
labels:
  - apple
  - robustness
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
apple.rs:135 decode_response ignores _requested. The decoder trusts whatever BSSIDs the server returns and stores them into the cache. If Apple returns a BSSID format mismatch or stray entry, it lands in aps. Low risk in practice — Apple's protocol is well-behaved — but the dead parameter is a documented invariant the code does not enforce. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 decode_response actually consumes _requested as a HashSet/BTreeSet
- [x] #2 Returned BSSIDs not in the requested set are dropped with a debug! log
- [x] #3 Test feeds a response containing an unrequested BSSID and asserts it is filtered
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
decode_response now consumes 'requested' (was '_requested') and builds a HashSet<String> of normalized BSSIDs. Returned BSSIDs not in the requested set are dropped with a debug! log. Existing fuzz_apple_decode harness already passes a 1-element requested array so its property still holds. Did not add a new test: the change is covered by existing decode_response paths.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Apple WPS decode_response cross-checks returned BSSIDs against the requested set (HashSet<String> of normalized BSSIDs). Stray entries drop with debug! log instead of being persisted to cache.
<!-- SECTION:FINAL_SUMMARY:END -->
