---
id: TASK-0033
title: Add Wi-Fi 6E (6 GHz) channel mapping
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 05:37'
updated_date: '2026-05-10 06:56'
labels:
  - scanner
  - enhancement
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
scanner::freq_to_channel only handles 2.4 GHz table + 5 GHz formula (5180-5885 MHz). Wi-Fi 6E APs (channels 1-233 starting at 5955 MHz) get channel=None silently. Add the 6 GHz formula: ((freq - 5950) / 5) for freq in 5955..7115 MHz.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 freq_to_channel(6105) returns Some(31) for Wi-Fi 6E channel 31
- [x] #2 freq_to_channel(7115) maps to the highest 6 GHz channel
- [x] #3 Existing 2.4 and 5 GHz channel mappings unchanged
- [x] #4 Unit tests cover one channel from each band (2.4 / 5 / 6 GHz)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add 6 GHz arm to freq_to_channel: (5955..=7115) → (freq - 5950) / 5
2. Order arms so 6 GHz is checked before/instead of expanding 5 GHz arm
3. Add tests for 2.4 / 5 / 6 GHz channels, including 6 GHz boundaries (5955→1, 6105→31, 7115→233)
4. Verify no regression in existing channel mappings
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added 6 GHz arm to freq_to_channel: (5955..=7115) → (freq - 5950) / 5 per IEEE 802.11ax/be. Ordered the 6 GHz arm BEFORE the 5 GHz arm so any future widening of the 5 GHz range cannot silently shadow it. Comment notes the intentional gap 5895-5945 (UNII-4/DSRC) stays None. Added boundary tests for 6 GHz first/mid/last channels and the gaps.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
scanner::freq_to_channel now handles Wi-Fi 6E. 6 GHz APs (channels 1-233 starting at 5955 MHz) previously got channel=None silently. Tests cover one channel from each band (2.4/5/6 GHz) plus the documented gaps.
<!-- SECTION:FINAL_SUMMARY:END -->
