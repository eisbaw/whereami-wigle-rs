---
id: TASK-0209
title: Add nix flake checks + a NixOS VM test for the module
status: To Do
assignee: []
created_date: '2026-07-02 04:53'
labels:
  - nix
  - testing
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
flake.nix has no checks output and package.nix has no passthru.tests, so nix flake check only evaluates - runs no tests, no clippy, never boots the module. There is no nixosTest covering the unit. Nothing automated would catch the world-readable DB, the DynamicUser/secrets crash, or the negative-int option crash - all boot-time behaviors a VM test surfaces immediately. Ref flake.nix:18-78, nix/package.nix:57-61.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 nix flake check fails on a clippy error or failing unit test
- [ ] #2 a NixOS VM test boots nixosModules.default, enables the service, and asserts it reaches active and answers on 127.0.0.1:4747
- [ ] #3 the VM test asserts DB/state-dir permissions are not world-readable
<!-- AC:END -->
