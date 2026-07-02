---
id: TASK-0210
title: Complete system-service systemd hardening (dangerous defaults left enabled)
status: To Do
assignee: []
created_date: '2026-07-02 04:53'
updated_date: '2026-07-02 23:06'
labels:
  - nix
  - security
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
module.nix serviceConfig omits PrivateDevices, SystemCallArchitectures, RestrictSUIDSGID, ProtectProc, ProcSubset, ProtectHostname, ProtectClock. Notably SystemCallFilter without SystemCallArchitectures=native leaves the non-native-ABI seccomp bypass open, and /dev is fully exposed though the daemon needs only netlink/AF_UNIX sockets. systemd-analyze security would flag each. Distinct from task-0176 (which only shares hardening between modules). Ref nix/module.nix:20-39.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 systemd-analyze security whereamid improves with no functional regression
- [ ] #2 SystemCallArchitectures=native is set alongside the syscall allow-list; PrivateDevices/RestrictSUIDSGID/ProtectProc/ProcSubset/ProtectHostname/ProtectClock added
- [ ] #3 Wi-Fi scanning still succeeds via both nmcli and iw under the tightened sandbox
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0197: nix/module.nix serviceConfig already gained UMask="0077" and StateDirectoryMode="0700" (for the location-history DB privacy). Do NOT double-add these when completing the systemd hardening; extend the remaining directives (e.g. ProtectProc, ProcSubset, SystemCallArchitectures, RestrictSUIDSGID, PrivateDevices, etc.) around them. hm-module.nix Service gained UMask="0077" too.
<!-- SECTION:NOTES:END -->
