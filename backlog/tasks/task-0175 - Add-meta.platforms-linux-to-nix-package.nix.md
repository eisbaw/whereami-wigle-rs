---
id: TASK-0175
title: Add meta.platforms = linux to nix/package.nix
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:35'
updated_date: '2026-07-09 08:11'
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
- [x] #1 nix/package.nix gains meta.platforms = lib.platforms.linux
- [x] #2 Optional: src/lib.rs or src/main.rs has #[cfg(not(target_os="linux"))] compile_error!("whereami daemon is Linux-only")
- [x] #3 Doc-comment in README/PRD: 'Linux-only by design (depends on nmcli/iw)'
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Done in-thread (small nix/doc change). AC#1: nix/package.nix meta gains platforms = platforms.linux (verified: nix eval .#default.meta.platforms -> the *-linux platform set). AC#2: whereamid/src/main.rs #[cfg(not(target_os="linux"))] compile_error! — inert on Linux (cargo build -p whereamid succeeds), fails fast with a clear message on other targets. AC#3: README tagline strengthened to "Linux-only by design" naming the nmcli/iw + systemd-socket dependency. Gate: just e2e green, clippy -D warnings clean, fmt clean. No runtime behavior change on Linux.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Declared whereami Linux-only at the packaging + compile level so a non-Linux contributor fails fast instead of at link time. nix/package.nix meta gains platforms = lib.platforms.linux; whereamid/main.rs adds a #[cfg(not(target_os="linux"))] compile_error! with a pointer to flake.nix/package.nix; README states Linux-only-by-design (depends on nmcli/iw + a systemd socket). Inert on Linux (build/tests unchanged); verified nix eval of meta.platforms + a clean e2e gate. Commit: (docs/build).
<!-- SECTION:FINAL_SUMMARY:END -->
