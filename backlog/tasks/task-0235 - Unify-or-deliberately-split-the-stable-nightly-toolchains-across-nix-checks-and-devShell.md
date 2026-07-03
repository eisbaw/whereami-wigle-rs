---
id: TASK-0235
title: >-
  Unify or deliberately split the stable/nightly toolchains across nix checks
  and devShell
status: To Do
assignee: []
created_date: '2026-07-03 07:33'
labels:
  - nix
  - tooling
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Review finding (cycle 9, mped M1, task-0209): the nix flake checks run nixpkgs STABLE rustc/clippy in crane's default RELEASE profile, while the devShell just-gate (just e2e/lint/fmt) runs fenix NIGHTLY in DEBUG. Same -D warnings flags, different lint sets (nightly clippy has more/different lints) and different runtime semantics (debug_assert compiled out, integer overflow wraps in release). The two gates can disagree; any disagreement fails loudly but wastes a cycle. Options: (a) share one toolchain: crane.mkLib(...).overrideToolchain with the fenix toolchain used by the devShell (makes checks nightly — nightly clippy churn hits CI); (b) pin devShell to the same stable as nixpkgs (loses cargo-fuzz which needs nightly - would need split shells); (c) keep the divergence but document it as deliberate defense-in-depth (two clippy channels catch more). Decide and implement; package.nix comment near cargoClippy references this task.
<!-- SECTION:DESCRIPTION:END -->
