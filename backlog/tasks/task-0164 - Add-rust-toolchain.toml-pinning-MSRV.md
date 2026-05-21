---
id: TASK-0164
title: Add rust-toolchain.toml pinning MSRV
status: To Do
assignee: []
created_date: '2026-05-20 21:32'
labels:
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F3. No rust-toolchain.toml. Cargo.toml has no rust-version. Flake uses fenix.latest.* (nightly tracking head). Non-Nix contributors get whatever stable they have. CI (when added) silently floats to new compilers.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add rust-toolchain.toml at workspace root pinning channel = 'stable', actual version (e.g. '1.83.0')
- [ ] #2 Add rust-version = '1.83' to whereamid/Cargo.toml AND whereami-client/Cargo.toml AND whereamid/fuzz/Cargo.toml
- [ ] #3 Add msrv = '1.83' to clippy.toml (or create one)
- [ ] #4 Keep nightly toolchain available in devShell for fuzz (named e.g. 'nightlyToolchain'); fuzz recipes use it via rustup run or a separate devShells.fuzz
- [ ] #5 Doc-comment in README explaining why MSRV is pinned
<!-- AC:END -->
