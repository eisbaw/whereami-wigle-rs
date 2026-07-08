---
id: TASK-0136
title: Deduplicate byte-identical build.rs across whereamid and whereami-client
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:22'
updated_date: '2026-07-08 21:04'
labels:
  - refactor
  - build
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
whereamid/build.rs and whereami-client/build.rs are byte-identical (~11 lines, compute GIT_REV from 'git rev-parse --short HEAD' with a GIT_REV file fallback). When the rev-stamping protocol changes (e.g. include build timestamp, add cargo metadata) both must change in lockstep. The architectural redundancy is that both daemon and CLI carry their own 'print git rev' code with no shared 'build banner' abstraction.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Option A: extract a whereami-build-utils crate exposing fn emit_git_rev_env(); both build.rs files become 'use whereami_build_utils::emit_git_rev_env; fn main() { emit_git_rev_env(); }'
- [ ] #2 Option B: shared via 'include\!("../whereamid/build.rs")' from the client
- [x] #3 Verify: nix build produces a binary with GIT_REV set; cargo build does too (via the in-tree git command)
- [x] #4 Doc-comment on the shared helper: explains the Nix preBuild path vs the git command path; either is authoritative
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Create whereami-build-utils crate (Cargo.toml + src/lib.rs) with pub fn emit_git_rev_env() holding the entire current build.rs logic (git_output, emit_existing_rerun_path, emit_git_rerun_paths + main body). Add AC#4 doc-comment covering both provisioning paths (git rev-parse vs GIT_REV file fallback).
2. Add whereami-build-utils to root Cargo.toml [workspace] members.
3. Replace both build.rs files with thin wrapper calling whereami_build_utils::emit_git_rev_env(); add [build-dependencies] entry to each crate Cargo.toml.
4. Verify: nix develop -c just e2e, cargo fmt --check, cargo build + version stamped, nix build .#default GIT_REV check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented Option A. Created whereami-build-utils crate; moved full build.rs logic into pub fn emit_git_rev_env() with AC#4 doc-comment covering both git rev-parse (dev) and GIT_REV-file (Nix preBuild) paths. Both build.rs are now thin wrappers; each crate got a [build-dependencies] entry. Root Cargo.toml members updated.

Gates (nix develop -c): just e2e = all tests pass (464+204+49+39+12+4+... 0 failed), clippy --all-targets -D warnings clean, cargo fmt --check clean.

Nix: nix build .#default builds (crane auto-picked up new crate). Built client reports cli: 0.6.2 (db88bd6-dirty) = flake dirtyShortRev, NOT unknown -> preBuild->GIT_REV file->env! chain intact. No flake.nix change needed.

GOTCHA: nix build initially failed (No such file whereami-build-utils/Cargo.toml) because flakes only copy git-TRACKED files into the store sandbox; the new crate was untracked. Fixed by git add (standard flake behavior, not a hack).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Deduplicated the two byte-identical build.rs files (Option A: shared build-dependency crate). Commit 4fcdd2e.

New whereami-build-utils crate exposes pub fn emit_git_rev_env() holding the full rev-stamping logic verbatim (git_output, the task-0229 rerun-if-changed emission for packed/unborn/detached refs, and the cargo:rustc-env=GIT_REV print with the GIT_REV-file fallback). Both whereamid/build.rs and whereami-client/build.rs are now 3-line wrappers calling it via a [build-dependencies] path entry; added to workspace members. AC#4 doc-comment explains both provisioning paths (git rev-parse for dev/cargo; the Nix preBuild GIT_REV file for the clean sandbox).

DECISION: Option A (shared crate), NOT Option B (include! the sibling build.rs). Review confirmed A is the right call — Option B is invisible to rust-analyzer, creates hidden directional coupling, and breaks if either crate moves; a build-dep makes the lockstep-critical shared contract explicit and tooling-visible. AC#2 (Option B) intentionally not taken.

KEY SUBTLETY preserved: a build script runs with CWD = the CONSUMING crate's dir even when the logic lives in a build-dependency, so the GIT_REV-file fallback still reads whereamid/GIT_REV / whereami-client/GIT_REV (exactly what package.nix preBuild writes) — the flake gitRev -> preBuild -> file -> env!(GIT_REV) -> daemon git_rev chain (pinned by nix/vm-test.nix:604) is intact; package.nix has ZERO diff; crane auto-picks-up the new crate (whole-workspace src + buildDepsOnly).

Verification: dual review gate. qa GO (777 tests 0 failed; clippy -D warnings + fmt clean; CARGO-path GIT_REV = real HEAD hash, not "unknown"). mped GO (byte-verbatim move confirmed vs original build.rs; crate build-only + hygienic; Nix chain preserved). NIX-path GIT_REV verified by the implementer: nix build .#default stamped a real dirty rev (db88bd6-dirty, could only come from the Nix file path since the sandbox has no .git). GOTCHA: a new workspace crate must be git-add-ed before nix build sees it (flakes copy only tracked files). Filed task-0260 (LOW: workspace version inheritance to collapse the now-triplicated 0.6.2 + publish=false on the internal crate).
<!-- SECTION:FINAL_SUMMARY:END -->
