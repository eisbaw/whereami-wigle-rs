---
id: TASK-0208
title: >-
  Document/guard that the home-manager user service can only scan via
  NetworkManager
status: To Do
assignee: []
created_date: '2026-07-02 04:53'
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
- [ ] #1 hm-module docs state that Wi-Fi scanning requires an active NetworkManager managing the interface
- [ ] #2 the nmcli-only limitation of the user service is discoverable without reading the source (assertion/warning or option text)
<!-- AC:END -->
