---
id: TASK-0234
title: >-
  NixOS VM test for the home-manager user service (self-bind socket 0600, DB
  perms)
status: To Do
assignee: []
created_date: '2026-07-03 07:11'
updated_date: '2026-07-03 09:09'
labels:
  - nix
  - testing
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0209 added a NixOS VM test for the SYSTEM module (nix/module.nix), but the home-manager user service (nix/hm-module.nix) is uncovered. Its security model differs: the daemon SELF-BINDS its socket under $XDG_RUNTIME_DIR (0600, user-owned, single-user by construction) instead of adopting a systemd socket-activated fd, and there is no whereami group / allowedUsers. Covering it needs a test-scoped home-manager flake input (the flake has none today) OR a user-session NixOS test that runs the systemd --user service; that new input is why it was deferred from 0209 as disproportionate. This is note-item (d) forward-carried from task-0196 and the hm half of task-0197's perms assertions.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A VM (or user-session) test runs the home-manager module's systemd --user whereamid service for a normal user and it reaches active
- [ ] #2 $XDG_RUNTIME_DIR/whereamid.sock is mode 0600 and owned by the user; 'whereami version --json' from that user returns ok:true
- [ ] #3 The daemon SELF-BINDS (journal logs 'control channel: unix socket <path>') and does NOT log 'adopting systemd socket-activated fd'
- [ ] #4 %h/.local/share/whereami is 0700 and aps.sqlite (+ -wal/-shm sidecars when present) are 0600 (hm variant of task-0197)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carry from task-0207/0208 batch (commits 0ae4eee, b50ab0e). The home-manager module gained config that CANNOT be evaluated in this repo (no home-manager flake input) — please assert these once the hm test infra exists:
- task-0208: hm-module.nix has `warnings = [ nmOnlyNote ]` (the NetworkManager-only scanning caveat). Assert it appears at hm evaluation/activation.
- task-0207: hm-module.nix has `assertions = import ./assertions.nix { inherit lib cfg; }` (debounceThreshold <= debounceWindow). Assert it FIRES under home-manager (set debounceThreshold=11, debounceWindow=10 -> hm eval fails) — mirror of the NixOS bite already proven in vm-test.nix.
- task-0208: options.nix wifiInterface description carries the NM caveat via wifiInterfaceExtraDoc for the hm module. Assert the rendered option doc contains it.
Both `warnings` and `assertions` are assumed valid hm options (mirrored from NixOS) but are UNVERIFIED here.
<!-- SECTION:NOTES:END -->
