---
id: TASK-0221
title: 'nmcli FREQ field parses as None (real output is ''5500 MHz'', not ''5500'')'
status: To Do
assignee: []
created_date: '2026-07-02 04:55'
labels:
  - math
  - correctness
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
parse_nmcli_output does fields[4].parse::<i32>() but nmcli terse mode emits e.g. '5500 MHz' (verified live), so frequency is None for every nmcli-scanned AP; the unit test feeds a bare '2462', pinning a format nmcli never produces. Near-zero impact today (ScannedNetwork.frequency is dead_code, channel comes from CHAN) but a latent bug that task-0123 (band-aware Channel) would inherit. Ref whereamid/src/scanner.rs:114,377.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 parse_nmcli_output extracts frequency:Some(5500) from a real ...:5500 MHz row
- [ ] #2 test fixtures use the actual nmcli terse format (NNNN MHz)
<!-- AC:END -->
