---
id: TASK-0175
title: Add meta.platforms = linux to nix/package.nix
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
Forge F16. flake.nix supportedSystems is Linux-only (correct: iw/networkmanager are Linux-only) but nix/package.nix has no meta.platforms. A macOS contributor can 'cargo check' happily until link time.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 nix/package.nix gains meta.platforms = lib.platforms.linux
- [ ] #2 Optional: src/lib.rs or src/main.rs has #[cfg(not(target_os="linux"))] compile_error!("whereami daemon is Linux-only")
- [ ] #3 Doc-comment in README/PRD: 'Linux-only by design (depends on nmcli/iw)'
<!-- AC:END -->
