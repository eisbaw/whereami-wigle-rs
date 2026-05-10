---
id: TASK-0033
title: Add Wi-Fi 6E (6 GHz) channel mapping
status: To Do
assignee: []
created_date: '2026-05-10 05:37'
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
- [ ] #1 freq_to_channel(6105) returns Some(31) for Wi-Fi 6E channel 31
- [ ] #2 freq_to_channel(7115) maps to the highest 6 GHz channel
- [ ] #3 Existing 2.4 and 5 GHz channel mappings unchanged
- [ ] #4 Unit tests cover one channel from each band (2.4 / 5 / 6 GHz)
<!-- AC:END -->
