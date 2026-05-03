---
id: TASK-16
title: NixOS package + module
status: Done
assignee: []
created_date: '2026-05-02 00:07'
updated_date: '2026-05-02 00:21'
labels: []
dependencies:
  - TASK-14
references:
  - PRD.md
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Create nix/package.nix: rustPlatform.buildRustPackage derivation. Wrap binary with makeWrapper to ensure iw is on PATH. Create nix/module.nix: NixOS module with services.whereami options (enable, bind, wifiInterface, wigle secrets, dailyLimit, all scan/debounce params). Generates systemd service with AmbientCapabilities=CAP_NET_ADMIN, DynamicUser, StateDirectory=whereami, hardened (NoNewPrivileges, ProtectSystem=strict, PrivateTmp).
<!-- SECTION:DESCRIPTION:END -->
