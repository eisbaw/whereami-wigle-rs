---
id: TASK-0220
title: freq_to_channel misses 6 GHz channel 2 (5935 MHz)
status: To Do
assignee: []
created_date: '2026-07-02 04:55'
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
- [ ] #1 freq_to_channel(5935) == Some(2)
- [ ] #2 doc comment notes the ch-2 special case; boundary test extended to include 5935
<!-- AC:END -->
