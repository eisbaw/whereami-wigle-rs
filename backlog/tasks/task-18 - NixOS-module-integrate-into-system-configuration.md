---
id: TASK-18
title: 'NixOS module: integrate into system configuration'
status: Done
assignee: []
created_date: '2026-05-03 21:25'
updated_date: '2026-05-03 21:30'
labels: []
dependencies: []
references:
  - PRD.md
  - nix/module.nix
  - nix/package.nix
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Create a proper NixOS module that can be imported into a system's configuration.nix or flake. The module should: (1) build whereamid via rustPlatform.buildRustPackage, (2) wrap binary with makeWrapper to ensure nmcli and iw are on PATH, (3) define services.whereami options matching all CLI args (bind, db, interface, scan intervals, debounce params, top-n, pending params, daily-limit, not-found-ttl, address-approx), (4) WiGLE credentials via a secrets file (not in nix store), (5) systemd service with AmbientCapabilities=CAP_NET_ADMIN, DynamicUser=true, StateDirectory=whereami, hardened (NoNewPrivileges, ProtectSystem=strict, PrivateTmp, ProtectHome=true), (6) test that the module evaluates without errors. Reference nix/module.nix and nix/package.nix stubs already in repo.
<!-- SECTION:DESCRIPTION:END -->
