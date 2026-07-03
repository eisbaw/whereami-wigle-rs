---
id: TASK-0208
title: >-
  Document/guard that the home-manager user service can only scan via
  NetworkManager
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-07-02 04:53'
updated_date: '2026-07-03 09:09'
labels:
  - nix
  - docs
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The scanner tries nmcli first then falls back to iw scan which needs CAP_NET_ADMIN. A systemd --user unit cannot be granted AmbientCapabilities, so the iw path can never succeed under the hm-module. On hosts where NetworkManager does not manage the Wi-Fi device (iwd, bare wpa_supplicant, headless), the hm-module daemon silently never scans - just logs 'wifi scan failed' forever. Nothing signals this hard dependency. Ref nix/hm-module.nix:17-31, whereamid/src/scanner.rs:21-31,165-166.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 hm-module docs state that Wi-Fi scanning requires an active NetworkManager managing the interface
- [x] #2 the nmcli-only limitation of the user service is discoverable without reading the source (assertion/warning or option text)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Batched with 0204-0208; see 0204 for shared VM strategy.

changes:
- hm-module.nix config: warnings = [ "<NM-only caveat>" ] stating Wi-Fi scanning under the user service REQUIRES NetworkManager actively managing the interface; the iw fallback needs CAP_NET_ADMIN which a systemd --user unit cannot be granted (no AmbientCapabilities), so on iwd/bare-wpa_supplicant/headless hosts the daemon silently never scans. Ref scanner.rs wifi_scan nmcli-then-iw.
- options.nix: new param wifiInterfaceExtraDoc ? "" appended to the wifiInterface description; hm-module passes the NM caveat, system module passes "" (keeps the false caveat off the system docs, which DO have CAP_NET_ADMIN via AmbientCapabilities). AC#1 (docs state requirement) + AC#2 (discoverable via warning + option text, no source reading).

LIMIT (honest): the flake has NO home-manager input, so hm-module warnings/assertions cannot be eval-tested in this repo (same reason 0234 was deferred from 0209). warnings/assertions are standard hm options (mirrored from NixOS) -> high confidence, but NOT machine-verified here. Forward-carry to 0234 to assert the warning at hm activation. No VM subtest here (system module iw works; the limitation is hm-specific).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carry from task-0209: the home-manager user service is now tracked for VM coverage in task-0234 (self-bind socket 0600, DB perms, self-bind vs socket-activation). Coordinate any NetworkManager-scanning guard assertion for the hm variant there — 0209 only covered the SYSTEM module.

Implemented (commit b50ab0e, docs(nix)).
- hm-module.nix: warnings = [ nmOnlyNote ] (shown at home-manager switch) stating Wi-Fi scanning REQUIRES NetworkManager actively managing the interface; the iw fallback needs CAP_NET_ADMIN which a systemd --user unit cannot get. Text defined once (nmOnlyNote), reused.
- options.nix: wifiInterfaceExtraDoc ? "" param appended to the wifiInterface description; hm passes the caveat, system passes "" (system has AmbientCapabilities=CAP_NET_ADMIN so iw works there -> no false caveat on system docs).

Verified: both files parse; nix eval of options.nix shows wifiInterface description = plain for system, "...\n\n<caveat>" for hm; nix flake check exit=0 (system module + VM unaffected by the new default-"" param).

LIMIT (honest): flake has NO home-manager input, so hm-module warnings/assertions are NOT eval-verified here (same reason 0234 was deferred). warnings/assertions are standard hm options (mirrored from NixOS) -> high confidence but unproven in this repo. No VM subtest (system iw works; limitation is hm-specific). Forward-carry appended to 0234 to assert the warning under a real hm eval.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Made the home-manager user service Wi-Fi-scan hard dependency on NetworkManager discoverable, instead of a silent "wifi scan failed" forever.

Changes:
- hm-module.nix: a warnings entry (shown at home-manager switch) states scanning requires NetworkManager actively managing the interface; the iw fallback needs CAP_NET_ADMIN which a systemd --user unit cannot get. Caveat text defined once and reused.
- options.nix: wifiInterfaceExtraDoc param (default "") appends the same caveat to the wifiInterface option docs for the hm module only; the system module (which has AmbientCapabilities=CAP_NET_ADMIN and can use iw) is unaffected.

Tests: both files parse; nix eval shows the wifiInterface description renders plain for system and with the caveat for hm; nix flake check green (system module + VM unaffected).

Limitation: no home-manager flake input, so the hm warnings/assertions are not evaluated here (verified option text satisfies both ACs; the warning is additional). Forward-carried to task-0234.
<!-- SECTION:FINAL_SUMMARY:END -->
