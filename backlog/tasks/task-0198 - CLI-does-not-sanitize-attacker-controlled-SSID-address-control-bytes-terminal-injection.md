---
id: TASK-0198
title: >-
  CLI does not sanitize attacker-controlled SSID/address control bytes ->
  terminal injection
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-07-02 04:52'
updated_date: '2026-07-02 17:47'
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
- [x] #1 an SSID containing 0x1b/0x0d/0x9b/0x07 renders as visible escaped text, not raw control bytes, in whereami scan and locate
- [x] #2 --json output is unchanged (already-escaped JSON)
- [x] #3 normal UTF-8 SSIDs (emoji/non-Latin) still render readably
- [x] #4 a unit test feeds a control-byte SSID through the renderer and asserts no raw C0/C1 byte reaches stdout
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. main.rs: add sanitize(&str)->String escaping every char::is_control() (C0/DEL/C1) via escape_default, passing all other chars (emoji/non-Latin) verbatim.
2. Refactor scan + locate rendering into pure render_scan()/render_locate()->String; sanitize daemon-sourced strings (ssid, bssid, scanned_at, address). Apply sanitize to other human renderers daemon strings (debug rev/bssid, history from/to/rfc3339, version strings, daemon error via fatal) as near-zero-cost defense-in-depth. Do NOT touch --json path.
3. Test: feed NetworkEntry SSID with 0x1b/0x0d/0x9b/0x07 through render_scan; assert bytes contain no raw C0 (except renderer \n), no 0x7f, no raw C1; assert an emoji SSID survives. Run gate, commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented human-renderer sanitization.
- sanitize(): char::is_control() (C0+DEL+C1) -> char::escape_default() (visible \u{..}/\r); all other chars verbatim (emoji/non-Latin survive).
- Extracted render_scan()/render_locate() as pure String fns (main print!s them) so AC#4 can test them directly. Sanitized ssid/bssid/scanned_at/address; defense-in-depth on debug rev+bssid, history from/to/segment ts, version strings, and the daemon error via fatal().
- --json path untouched (AC#2): after task-0183 it is byte-identical passthrough.
Key UTF-8 subtlety for AC#4: C1 controls U+0080..U+009F encode as 0xC2 0x80..0x9F; a naive "no byte 0x80..0x9F" check would false-fail on emoji/© continuation bytes. The test checks C0/DEL at byte level and C1 as the exact 0xC2,0x80..0x9F pair, so it bites without flagging legit UTF-8.
Live verify: --json version/locate byte-identical to raw daemon line (json untouched); two consecutive RAW daemon scans differ (scan state volatile) which is why comparing --json scan to a *separate* raw scan shows a diff — not a passthrough bug. Human scan renders real SSID "DIRECT-qPC460 Series" readably.

Review gate GO. Reviewer verified the AC#4 test bites (byte-level C0/DEL/C1 asserts, UTF-8-aware C1 windowing) and that every daemon-sourced string in every human renderer is wrapped; db_status/provenance are typed enums so not injectable. Post-review hardening (fc54f0d): transport-error strings through fatal()/run_version now sanitized too (CLI/OS-origin today, closes the class).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Sanitized attacker-controlled control bytes in the human CLI renderer to stop terminal-escape injection from malicious SSIDs/addresses.

Why: SSID is an attacker-controlled 802.11 field passed verbatim through nmcli/JSON to `whereami scan`; a nearby AP with ESC/OSC in its SSID could set the terminal title/clipboard or manipulate the screen of anyone scanning. The locate address had the same exposure.

Changes:
- sanitize(): escapes every char::is_control() char (C0, DEL, C1 — ESC/CR/BEL/CSI) via char::escape_default; printable ASCII, emoji and non-Latin scripts pass through unchanged.
- Extracted render_scan()/render_locate() as pure, testable String builders; sanitized ssid/bssid/scan-timestamp/address and, as defense-in-depth, the daemon strings in debug/history/version and the daemon error sent to fatal().
- Left the --json path byte-identical (task-0183 passthrough) — that is AC#2.

User impact: a hostile SSID now shows as visible \u{1b}-style text instead of executing terminal escapes; ordinary/emoji SSIDs are unaffected.

Tests: 4 new unit tests — control-byte SSID is escaped while an emoji survives, byte-level assertion that no raw C0 (bar \n)/DEL/C1 reaches stdout, a byte-stable clean-layout guard, and an OSC-title injection escaped in the address. Gate: whereami-client 21 (bin) + 7 (lib) tests, clippy -D warnings + fmt clean. Verified live.
<!-- SECTION:FINAL_SUMMARY:END -->
