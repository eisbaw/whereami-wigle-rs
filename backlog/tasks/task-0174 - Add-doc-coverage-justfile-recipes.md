---
id: TASK-0174
title: Add doc/coverage justfile recipes
status: To Do
assignee: []
created_date: '2026-05-20 21:35'
labels:
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F12. No 'cargo doc --no-deps --open', no 'cargo llvm-cov' recipe. Coverage would directly support task-0035 (integration tests) and task-0041 (replace vanity proptests).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add pkgs.cargo-llvm-cov to devShell
- [ ] #2 Justfile: 'just doc' = cargo doc --no-deps --workspace --open
- [ ] #3 Justfile: 'just cov' = cargo llvm-cov --workspace --html (produces report under target/llvm-cov)
- [ ] #4 CI workflow (task-0163) can optionally upload coverage report as artifact
<!-- AC:END -->
