---
id: TASK-0154
title: 'Test: freq_to_channel boundary table'
status: Done
assignee: []
created_date: '2026-05-20 21:30'
updated_date: '2026-07-05 08:37'
labels:
  - tests
  - scanner
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P0-7. scanner.rs:434 tests 7 hand-picked frequencies. Kani (line 461) proves panic-freedom. No test pins boundaries at 5180 / 5185 / 5885 / 5890 / 5950 / 5955 / 7115 / 7120. Off-by-one in range guards (e.g. 5185..=5885) would slip through. 5905 (UNII-4 gap) mapping to a 5GHz channel would silently mis-classify.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Parametric test over every band boundary:
- [x] #2 assert_eq!(freq_to_channel(2412), Some(1)); assert_eq!(freq_to_channel(2407), None); assert_eq!(freq_to_channel(2484), Some(14)); assert_eq!(freq_to_channel(2489), None);
- [x] #3 assert_eq!(freq_to_channel(5180), Some(36)); assert_eq!(freq_to_channel(5175), None); assert_eq!(freq_to_channel(5885), Some(177));
- [x] #4 assert_eq!(freq_to_channel(5955), Some(1)); assert_eq!(freq_to_channel(5950), None); assert_eq!(freq_to_channel(7115), Some(233)); assert_eq!(freq_to_channel(7120), None);
- [x] #5 Property: every Some(channel) output is in 1..=233
- [x] #6 Composes with task-0123 (Channel band-aware enum) — when that lands, replace Option<i32> with Option<Channel>
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Replace the hand-picked frequency test with a table that pins each specified band edge and adjacent rejection.
2. Add an exhaustive property check that every mapped channel is within 1..=233.
3. Preserve the existing exact interior mapping assertions.
4. Run focused tests and the full just e2e gate.
5. Obtain parallel QA and architecture reviews, resolve findings, then commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
First review gate: NO-GO. Architecture found the rewrite dropped exact interior mappings (2437→6, 5825→165, 6105→31). QA found ±1 MHz range-guard mutations were not caught. Corrective rows required before re-review.

Corrective review gate: GO from both qa-test-runner and mped-architect. The final table preserves interior mappings and catches narrowing/widening at every continuous guard; the deterministic u16 sweep covers every current Some-producing branch.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced the hand-picked scanner frequency test with a table covering required ±5 MHz boundaries, immediate ±1 MHz guard sentinels, UNII-4 rejection, and retained representative interior mappings. Added a deterministic exhaustive codomain check across the complete current Some-producing domain. Production mapping is unchanged; 5935 MHz remains for dependent TASK-0220. Full E2E: 493 tests passed and strict clippy clean. Mandatory QA and architecture reviews: GO.
<!-- SECTION:FINAL_SUMMARY:END -->
