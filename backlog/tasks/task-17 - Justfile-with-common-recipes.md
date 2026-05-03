---
id: TASK-17
title: Justfile with common recipes
status: Done
assignee: []
created_date: '2026-05-02 00:07'
updated_date: '2026-05-02 00:13'
labels: []
dependencies:
  - TASK-1
references:
  - PRD.md
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Create Justfile with recipes: build (cargo build), test (cargo test), run (cargo run -- with sensible dev defaults), lint (cargo clippy), fmt (cargo fmt --check), clean (cargo clean). All recipes run inside nix-shell if shell.nix is present.
<!-- SECTION:DESCRIPTION:END -->
