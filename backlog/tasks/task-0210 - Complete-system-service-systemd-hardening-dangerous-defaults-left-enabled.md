---
id: TASK-0210
title: Complete system-service systemd hardening (dangerous defaults left enabled)
status: To Do
assignee: []
created_date: '2026-07-02 04:53'
updated_date: '2026-07-03 09:09'
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

Forward-carry from task-0204/0205 batch (commits f6bb2da, effa4a8). nix/module.nix systemd.services.whereamid now carries, around the serviceConfig you will harden:
- UNIT-level: startLimitIntervalSec=60, startLimitBurst=5 (task-0204). These are top-level service attrs, NOT serviceConfig.
- serviceConfig: Restart="on-failure", RestartSec=2 (task-0204).
- serviceConfig is now `{ ...base... } // lib.optionalAttrs (cfg.credentialsFile != null) { LoadCredential = [ "config:${cfg.credentialsFile}" ]; }` (task-0205). ADD your hardening directives (PrivateDevices, SystemCallArchitectures=native, RestrictSUIDSGID, ProtectProc, ProcSubset, ProtectHostname, ProtectClock, etc.) to the BASE attrset before the //, or they will sit oddly next to the conditional merge.
- config also has `assertions = import ./assertions.nix {...}` (task-0207).
GOTCHA for your VM verification: with Restart=on-failure the daemon RESTARTS on failure; if a too-tight sandbox directive makes it exit non-zero it will crash-loop until startLimitBurst (5) then land in `failed`. The vm-test nonloopback node already relies on that; watch that AC#3 (scanning still works via nmcli AND iw) does not silently regress into a restart loop.
<!-- SECTION:NOTES:END -->
