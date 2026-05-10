---
id: TASK-0043
title: Handle nmcli empty signal field explicitly
status: To Do
assignee: []
created_date: '2026-05-10 05:39'
labels:
  - scanner
  - data-quality
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
scanner.rs nmcli parsing falls back to signal=0% on a missing/malformed signal field, then converts via the linear formula to signal_dbm = -90. This is indistinguishable from a real weak AP and feeds garbage weights into trilateration. Either skip rows with no signal or model signal as Option<i32>. Also: the linear conversion (-90 + signal_pct*60/100) is documented to be inaccurate vs. iw's dBm; consider preferring iw exclusively when accuracy matters or document the limitation in PRD.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Empty/missing nmcli signal field produces a skipped row OR a None signal, not a fake -90 dBm
- [ ] #2 Existing parser tests cover the 'empty signal' input shape
- [ ] #3 PRD or scanner.rs comment documents the nmcli vs iw signal accuracy limitation
<!-- AC:END -->
