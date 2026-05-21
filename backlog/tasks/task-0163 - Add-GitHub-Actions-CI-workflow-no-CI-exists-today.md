---
id: TASK-0163
title: Add GitHub Actions CI workflow (no CI exists today)
status: To Do
assignee: []
created_date: '2026-05-20 21:32'
labels:
  - tooling
  - ci
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F1. There is no CI. .github/workflows/ does not exist. just qa is local convention; nothing prevents a PR landing with broken tests, clippy warnings, formatting drift, or fuzz regression. The whole e2e/qa apparatus is on the honor system.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add .github/workflows/ci.yml triggering on push + PR
- [ ] #2 Matrix: x86_64-linux (and optionally aarch64-linux via cross or QEMU)
- [ ] #3 Job runs: nix develop -c just qa (cargo test + clippy -D warnings + fmt --check + 15s fuzz smoke)
- [ ] #4 Use cachix/install-nix-action + DeterminateSystems/magic-nix-cache for nix store caching; cache ~/.cargo/registry separately
- [ ] #5 Separate weekly workflow runs just fuzz-all (longer fuzz cadence)
- [ ] #6 Add status badge to README
- [ ] #7 Optional: weekly cargo-audit + cargo-deny (depends on task-0168)
<!-- AC:END -->
