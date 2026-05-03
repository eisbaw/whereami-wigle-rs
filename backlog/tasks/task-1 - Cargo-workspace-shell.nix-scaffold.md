---
id: TASK-1
title: Cargo workspace + shell.nix scaffold
status: Done
assignee: []
created_date: '2026-05-02 00:05'
updated_date: '2026-05-02 00:09'
labels: []
dependencies: []
references:
  - PRD.md
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Create Cargo.toml workspace with whereamid (bin) and whereami-client (lib) crates. Create shell.nix with rustc, cargo, pkg-config, openssl, sqlite, iw. Ensure cargo build works in nix-shell. No flake. Minimal main.rs stubs.
<!-- SECTION:DESCRIPTION:END -->
