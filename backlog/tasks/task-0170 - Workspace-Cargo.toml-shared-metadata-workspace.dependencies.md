---
id: TASK-0170
title: 'Workspace Cargo.toml: shared metadata + workspace.dependencies'
status: To Do
assignee: []
created_date: '2026-05-20 21:34'
labels:
  - tooling
  - cargo
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F8. Top-level Cargo.toml is two lines (workspace + members). Every dep version (tokio, reqwest, serde, clap, anyhow, chrono) is duplicated in whereamid/ and whereami-client/. clap features already differ between them; future drift goes unnoticed.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Top-level Cargo.toml gains [workspace.package] with version, edition, license, repository, rust-version
- [ ] #2 Top-level Cargo.toml gains [workspace.dependencies] with serde, serde_json, anyhow, clap (without member-specific features), chrono
- [ ] #3 Member Cargo.tomls use 'package.version.workspace = true', 'serde.workspace = true', 'serde.features = ["derive"]' etc.
- [ ] #4 Single-source-of-truth for the version bump (composes with task-0169)
- [ ] #5 Verify: cargo build succeeds; Cargo.lock unchanged in terms of resolved versions
<!-- AC:END -->
