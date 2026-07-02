---
id: TASK-0198
title: >-
  CLI does not sanitize attacker-controlled SSID/address control bytes ->
  terminal injection
status: To Do
assignee: []
created_date: '2026-07-02 04:52'
labels:
  - security
  - cli
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
run_scan/run_locate println! the SSID and address raw. SSID is an attacker-controlled 802.11 field: parse_nmcli_output takes it verbatim (nmcli terse escapes only : and backslash, not C0/C1 like ESC 0x1B), and the JSON transport is lossless for control bytes (serde emits \u001b, client decodes back to raw 0x1B). A malicious nearby AP whose SSID contains terminal escapes (OSC title/clipboard set, screen manipulation) attacks the terminal of anyone running whereami scan. --json is safe (re-escaped); only the human renderer is affected. Ref whereami-client/src/main.rs:206-227,181-204, lib.rs:346-373, whereamid/src/scanner.rs:97-104.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 an SSID containing 0x1b/0x0d/0x9b/0x07 renders as visible escaped text, not raw control bytes, in whereami scan and locate
- [ ] #2 --json output is unchanged (already-escaped JSON)
- [ ] #3 normal UTF-8 SSIDs (emoji/non-Latin) still render readably
- [ ] #4 a unit test feeds a control-byte SSID through the renderer and asserts no raw C0/C1 byte reaches stdout
<!-- AC:END -->
