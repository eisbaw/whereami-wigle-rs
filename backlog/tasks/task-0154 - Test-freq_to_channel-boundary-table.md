---
id: TASK-0154
title: 'Test: freq_to_channel boundary table'
status: To Do
assignee: []
created_date: '2026-05-20 21:30'
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
- [ ] #1 Parametric test over every band boundary:
- [ ] #2 assert_eq!(freq_to_channel(2412), Some(1)); assert_eq!(freq_to_channel(2407), None); assert_eq!(freq_to_channel(2484), Some(14)); assert_eq!(freq_to_channel(2489), None);
- [ ] #3 assert_eq!(freq_to_channel(5180), Some(36)); assert_eq!(freq_to_channel(5175), None); assert_eq!(freq_to_channel(5885), Some(177));
- [ ] #4 assert_eq!(freq_to_channel(5955), Some(1)); assert_eq!(freq_to_channel(5950), None); assert_eq!(freq_to_channel(7115), Some(233)); assert_eq!(freq_to_channel(7120), None);
- [ ] #5 Property: every Some(channel) output is in 1..=233
- [ ] #6 Composes with task-0123 (Channel band-aware enum) — when that lands, replace Option<i32> with Option<Channel>
<!-- AC:END -->
