---
id: TASK-0123
title: 'Channel must be band-aware: TwoFourGhz/FiveGhz/SixGhz enum'
status: To Do
assignee: []
created_date: '2026-05-20 21:18'
labels:
  - correctness
  - domain
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #2. Wi-Fi 6E/7 added 6 GHz operation where channel numbers restart at 1, overlapping 2.4 GHz. Channel 1@2412MHz and Channel 1@5955MHz are DIFFERENT physical channels. (channel, band) is the actual identifier. scanner.rs:17 stores channel as Option<i32>; db.rs:17 + SQL 'aps.channel INTEGER' has no band. freq_to_channel (scanner.rs:302) returns the IEEE-correct number per band but the type immediately erases the band. After parsing, channel=Some(1) could mean 2.4 GHz ch1 OR 6 GHz ch1.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Replace channel: Option<i32> with channel: Option<Channel> where enum Channel { TwoFourGhz(u8), FiveGhz(u16), SixGhz(u16) }
- [ ] #2 Channel implements Display showing band + number (e.g. '6G/37')
- [ ] #3 freq_to_channel returns Option<Channel> with the right variant per frequency range
- [ ] #4 Schema migration v6: aps.channel column either dropped (if frequency is the canonical source) OR widened to TEXT carrying the band-prefixed string
- [ ] #5 Unit tests: freq_to_channel(2412) -> TwoFourGhz(1); freq_to_channel(5955) -> SixGhz(1); freq_to_channel(5180) -> FiveGhz(36)
- [ ] #6 Database tests verify round-trip (insert Channel, read back same variant)
<!-- AC:END -->
