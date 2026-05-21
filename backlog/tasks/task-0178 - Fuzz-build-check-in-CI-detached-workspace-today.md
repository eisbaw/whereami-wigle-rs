---
id: TASK-0178
title: Fuzz-build check in CI (detached workspace today)
status: To Do
assignee: []
created_date: '2026-05-20 21:36'
labels:
  - tooling
  - fuzz
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F6. whereamid/fuzz/Cargo.toml declares its own [workspace] (cargo-fuzz default). Top-level cargo build/test/clippy --all-targets never touch fuzz targets. A fuzz target failing to compile after a whereamid lib refactor is only caught by 'just fuzz' locally — and CI doesn't run that today.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Justfile: 'just fuzz-build' = cd whereamid && cargo +nightly fuzz build (compile-only check)
- [ ] #2 CI workflow (task-0163) runs 'just fuzz-build' on every push — fast, catches the realistic regression that a refactor broke the fuzz target's compile
- [ ] #3 Separate from the longer fuzz-execution recipe ('just fuzz' = 15s/target smoke)
<!-- AC:END -->
