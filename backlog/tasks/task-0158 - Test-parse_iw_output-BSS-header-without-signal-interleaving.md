---
id: TASK-0158
title: 'Test: parse_iw_output BSS-header-without-signal interleaving'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:31'
updated_date: '2026-07-09 04:20'
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
- [x] #1 Input: BSS#1 has BSSID + SSID + channel but NO signal; BSS#2 has BSSID + signal
- [x] #2 Assert BSS#2 in result has SSID=None, channel=None (no cross-record leakage)
- [x] #3 Symmetric for nmcli parser if structurally similar
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Confirm existing malformed-signal test does NOT cover the leak-from-dropped-BSS#1 direction
2. Add test: BSS#1 has bssid+SSID+freq+channel but NO signal (dropped); BSS#2 has only bssid+signal
3. Assert BSS#2 ssid/channel/frequency are its own None (no cross-AP bleed)
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added parse_iw_output_dropped_bss_does_not_leak_fields_into_next (scanner.rs): BSS#1 has ssid/channel/freq but NO signal (dropped); BSS#2 has only bssid+signal; asserts BSS#2 ssid/channel/frequency all None (AC#1/#2). BITES: removing the else-branch field reset (scanner.rs:912-915) makes BSS#2 inherit BSS#1 ssid=leaky-ssid/freq=5180/channel -> all 3 asserts fail (reviewer-confirmed). Complements ..._drops_bss_with_malformed_signal (opposite direction). AC#3 (nmcli symmetry): the nmcli parser is structurally different (line-oriented CSV, no BSS-block accumulator), so the cross-record-leak hazard does not apply there — the iw block accumulator is the only leak-prone parser. Dual gate GO/GO.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Pinned that the iw parser does not leak a dropped BSS block\x27s fields into the next record (silent cross-AP corruption). New test: a no-signal BSS#1 (dropped) must not pass its ssid/channel/freq to BSS#2. Bite-verified by removing the accumulator reset. nmcli is structurally immune (no block accumulator). Test-only; commit 344fbb9.
<!-- SECTION:FINAL_SUMMARY:END -->
