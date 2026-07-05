---
id: TASK-0210
title: Complete system-service systemd hardening (dangerous defaults left enabled)
status: Done
assignee: []
created_date: '2026-07-02 04:53'
updated_date: '2026-07-05 03:41'
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
- [x] #1 systemd-analyze security whereamid improves with no functional regression
- [x] #2 SystemCallArchitectures=native is set alongside the syscall allow-list; PrivateDevices/RestrictSUIDSGID/ProtectProc/ProcSubset/ProtectHostname/ProtectClock added
- [x] #3 Wi-Fi scanning still succeeds via both nmcli and iw under the tightened sandbox
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add the missing system-service hardening directives to the base serviceConfig, preserving capabilities, address families, state-directory privacy, restart policy, and conditional LoadCredential.
2. Extend the NixOS VM harness to assert the generated unit properties and a bounded systemd-analyze security exposure score.
3. Add deterministic end-to-end VM scan probes for both the nmcli-success path and the iw fallback, including proof that the iw child retains CAP_NET_ADMIN inside the tightened sandbox.
4. Build/run focused Nix evaluation and VM checks, then run the full nix-shell E2E gate.
5. Run mandatory parallel QA and architecture review, fix every blocker, complete the tracker, rerun E2E, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0197: nix/module.nix serviceConfig already gained UMask="0077" and StateDirectoryMode="0700" (for the location-history DB privacy). Do NOT double-add these when completing the systemd hardening; extend the remaining directives (e.g. ProtectProc, ProcSubset, SystemCallArchitectures, RestrictSUIDSGID, PrivateDevices, etc.) around them. hm-module.nix Service gained UMask="0077" too.

Forward-carry from task-0204/0205 batch (commits f6bb2da, effa4a8). nix/module.nix systemd.services.whereamid now carries, around the serviceConfig you will harden:
- UNIT-level: startLimitIntervalSec=60, startLimitBurst=5 (task-0204). These are top-level service attrs, NOT serviceConfig.
- serviceConfig: Restart="on-failure", RestartSec=2 (task-0204).
- serviceConfig is now `{ ...base... } // lib.optionalAttrs (cfg.credentialsFile != null) { LoadCredential = [ "config:${cfg.credentialsFile}" ]; }` (task-0205). ADD your hardening directives (PrivateDevices, SystemCallArchitectures=native, RestrictSUIDSGID, ProtectProc, ProcSubset, ProtectHostname, ProtectClock, etc.) to the BASE attrset before the //, or they will sit oddly next to the conditional merge.
- config also has `assertions = import ./assertions.nix {...}` (task-0207).
GOTCHA for your VM verification: with Restart=on-failure the daemon RESTARTS on failure; if a too-tight sandbox directive makes it exit non-zero it will crash-loop until startLimitBurst (5) then land in `failed`. The vm-test nonloopback node already relies on that; watch that AC#3 (scanning still works via nmcli AND iw) does not silently regress into a restart loop.

Implemented the seven requested base-service restrictions without broadening the existing capability, address-family, device, or syscall policy. The NixOS VM now compares the effective hardened unit against an otherwise-identical pre-TASK-0210 control, asserts exact runtime/unit properties, and exercises both real daemon scan branches. The nmcli fixture is gated on a copied real nmcli successfully reaching NetworkManager over D-Bus. The iw fixture requires exact child CapEff=0x1000 and a real AF_NETLINK/NETLINK_GENERIC socket before emitting data. Both paths are observed through whereami scan --json with a stable daemon PID and NRestarts=0. A pre-existing tooling defect was discovered separately: shell.nix pins a flake-compat archive URL that returns HTTP 404, so legacy nix-shell cannot evaluate; the locked native nix develop shell was used for all gates and the defect will be filed separately.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Completed system-service sandbox hardening and end-to-end VM proof. systemd-analyze summed exposure improved from 3.600 to 2.500. Runtime retained only CAP_NET_ADMIN, AF_INET/AF_INET6/AF_NETLINK/AF_UNIX, and the unchanged 390-syscall allowlist. Both mandatory QA and architecture gates returned GO after fixing the review findings: an exact removeAttrs-only baseline and an explicit RestrictSUIDSGID=true unit pin. Corrected-tree validation passed multiple independent 14-subtest VM rebuilds, 417 E2E tests, strict clippy, formatting, and diff checks.
<!-- SECTION:FINAL_SUMMARY:END -->
