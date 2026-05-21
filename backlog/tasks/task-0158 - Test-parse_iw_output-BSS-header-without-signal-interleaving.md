---
id: TASK-0158
title: 'Test: parse_iw_output BSS-header-without-signal interleaving'
status: To Do
assignee: []
created_date: '2026-05-20 21:31'
labels:
  - tests
  - scanner
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P1-4. scanner.rs:221. The parser commits only when (bssid, signal) are both Some. The 'previous entry has bssid but no signal' branch at lines 242-246 discards. If someone simplifies it to leave the previous SSID/channel/freq in place, BSS#2 inherits BSS#1's SSID — silent cross-AP data corruption.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Input: BSS#1 has BSSID + SSID + channel but NO signal; BSS#2 has BSSID + signal
- [ ] #2 Assert BSS#2 in result has SSID=None, channel=None (no cross-record leakage)
- [ ] #3 Symmetric for nmcli parser if structurally similar
<!-- AC:END -->
