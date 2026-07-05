---
id: TASK-0221
title: 'nmcli FREQ field parses as None (real output is ''5500 MHz'', not ''5500'')'
status: Done
assignee: []
created_date: '2026-07-02 04:55'
updated_date: '2026-07-05 08:52'
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
- [x] #1 parse_nmcli_output extracts frequency:Some(5500) from a real ...:5500 MHz row
- [x] #2 test fixtures use the actual nmcli terse format (NNNN MHz)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Force LC_ALL=C on both nmcli subprocesses so scripted output has a stable locale.
2. Parse the real C-locale FREQ representation (`NNNN MHz`) without changing row acceptance behavior.
3. Convert every nmcli row fixture, including the VM mock, to actual terse-output format and assert 5500 MHz becomes Some(5500).
4. Preserve malformed-field behavior and legacy bare-integer compatibility.
5. Run focused tests and exact just e2e, then obtain parallel QA and architecture reviews and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
First review gate: QA GO; architecture NO-GO. The parser assumed an English ` MHz` suffix without forcing nmcli to C locale. Required correction: set LC_ALL=C on both nmcli commands, describe the helper as C-locale parsing, and enforce the invariant in the VM mock.

Corrective review gate: GO from qa-test-runner and mped-architect. Both nmcli child commands now force C locale, the VM mock enforces that invariant, and Nix parse plus strict VM derivation evaluation pass. Full VM boot was judged unnecessary because frequency is not exposed by its scan JSON assertion and no VM wiring changed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Fixed real nmcli frequency parsing by accepting the C-locale `NNNN MHz` representation while preserving bare-number compatibility and rejecting other suffixes. Both nmcli subprocesses now set child-local LC_ALL=C. Updated all scanner and VM fixtures, added explicit 5500 and malformed-row coverage, and made the VM mock enforce locale. Full E2E passed 495 tests with strict clippy; QA and architecture reviews returned GO.
<!-- SECTION:FINAL_SUMMARY:END -->
