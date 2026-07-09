---
id: TASK-0265
title: Fix nix/package.nix version read broken by workspace version inheritance
status: Done
assignee:
  - '@myself'
created_date: '2026-07-09 00:00'
updated_date: '2026-07-09 00:11'
labels:
  - nix
  - build
  - regression
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0260 set whereamid/Cargo.toml to 'version.workspace = true', but nix/package.nix line 41 still reads 'cargoToml.package.version' from whereamid/Cargo.toml. That TOML value now parses to the set { workspace = true; } instead of a string, so 'nix build .#default' and 'nix flake check' fail with 'cannot coerce a set to a string'. Regression is invisible to 'just e2e' because e2e never runs nix build. Root cause: the concrete version now lives in the root Cargo.toml [workspace.package].version. Fix: read the version from the workspace Cargo.toml.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 nix/package.nix reads the package version from the root [workspace.package].version so it resolves to the string "0.6.2", not a set
- [x] #2 'nix eval .#default.version' returns the version string without a coercion error
- [x] #3 'nix build .#default' proceeds past evaluation (builds or fails only for unrelated reasons)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Root cause: task-0260 set whereamid/Cargo.toml to version.workspace=true; nix/package.nix line 41 still read cargoToml.package.version from that file, which parses to { workspace = true; } (a set), not a string. Invisible to just e2e (never runs nix build).

Fix: read workspaceToml.workspace.package.version from the root Cargo.toml.

Verified:
- nix eval --raw .#default.version -> 0.6.2 (was: cannot coerce a set to a string).
- nix build .#default -> succeeds, result/bin/whereamid --version prints "whereamid 0.6.2". Build also transitively compiled the new whereami-protocol crate (crane whole-workspace src), confirming task-0114 crate packaging works too.

GOTCHA feed-forward: any future crate that inherits version.workspace=true will break any Nix expr that reads package.version directly from its Cargo.toml. Read from [workspace.package] in the root Cargo.toml.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Fixed nix build/flake check, broken since task-0260 by workspace version inheritance.

nix/package.nix read the version from whereamid/Cargo.toml, but that file now has version.workspace=true, so the value parsed to the set { workspace = true; } and Nix aborted with "cannot coerce a set to a string". just e2e never exercises nix build, so it slipped past the gate.

Change: read the concrete version from the root Cargo.toml [workspace.package].version.

Verified end-to-end: nix eval .#default.version -> 0.6.2; nix build .#default succeeds and yields whereamid 0.6.2.

Commit: 97b1492.
<!-- SECTION:FINAL_SUMMARY:END -->
