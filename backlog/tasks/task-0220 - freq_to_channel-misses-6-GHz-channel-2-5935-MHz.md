---
id: TASK-0220
title: freq_to_channel misses 6 GHz channel 2 (5935 MHz)
status: Done
assignee: []
created_date: '2026-07-02 04:55'
updated_date: '2026-07-05 08:42'
labels:
  - math
dependencies:
  - TASK-0154
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
freq_to_channel maps 5955..=7115 via (f-5950)/5 and returns None for 5935. Per IEEE 802.11ax-2021 Annex E, 6 GHz channel 2 is a 20 MHz channel centred at 5935 MHz - the single exception to the (freq-5950)/5 rule. A 6E AP beaconing on channel 2 gets channel:None. Metadata only (channel is not used in positioning) but a concretely wrong table entry in done task-0033. Ref whereamid/src/scanner.rs:318-323.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 freq_to_channel(5935) == Some(2)
- [x] #2 doc comment notes the ch-2 special case; boundary test extended to include 5935
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add the IEEE 6 GHz channel-2 exception mapping 5935 MHz to channel 2 without widening any range guard.
2. Update the frequency conversion documentation and boundary table, including immediate sentinels around the isolated exception.
3. Run focused tests and exact just e2e.
4. Obtain parallel QA and architecture reviews, resolve every finding, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation frozen and reviewed at diff hash 34db2cd5511e2683100cd503c7e06b712cf211d3476ebd679070ffa3a009ff4c. Mandatory qa-test-runner and mped-architect reviews both returned GO; 5934/5936 sentinels prove the exception remains isolated.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Mapped the isolated 6 GHz channel-2 center frequency 5935 MHz to channel 2 with an exact match, leaving normal frequency ranges unchanged. Updated function and gap documentation and extended the boundary table with 5934/5935/5936. Full E2E passed 493 tests with strict clippy; mandatory QA and architecture reviews returned GO.
<!-- SECTION:FINAL_SUMMARY:END -->
