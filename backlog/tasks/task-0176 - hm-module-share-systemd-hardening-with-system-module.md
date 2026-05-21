---
id: TASK-0176
title: 'hm-module: share systemd hardening with system module'
status: To Do
assignee: []
created_date: '2026-05-20 21:35'
labels:
  - tooling
  - nix
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F17. nix/hm-module.nix:18-26 user service has only Type/ExecStart/Restart/RestartSec. Missing Protect*/SystemCallFilter from nix/module.nix:24-39. Same binary, same threat model.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Extract serviceHardening = { ... } attrset into nix/service-hardening.nix
- [ ] #2 Both nix/module.nix and nix/hm-module.nix import + apply it
- [ ] #3 Options that don't apply to user units (DynamicUser, NoNewPrivileges sometimes) are gated by an 'isSystem' bool
- [ ] #4 NixOS test still passes (if any); home-manager dry-build still succeeds
<!-- AC:END -->
