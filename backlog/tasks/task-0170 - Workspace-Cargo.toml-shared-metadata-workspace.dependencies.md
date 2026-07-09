---
id: TASK-0170
title: 'Workspace Cargo.toml: shared metadata + workspace.dependencies'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:34'
updated_date: '2026-07-09 08:18'
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
- [x] #1 Top-level Cargo.toml gains [workspace.package] with version, edition, license, repository, rust-version
- [x] #2 Top-level Cargo.toml gains [workspace.dependencies] with serde, serde_json, anyhow, clap (without member-specific features), chrono
- [x] #3 Member Cargo.tomls use 'package.version.workspace = true', 'serde.workspace = true', 'serde.features = ["derive"]' etc.
- [x] #4 Single-source-of-truth for the version bump (composes with task-0169)
- [x] #5 Verify: cargo build succeeds; Cargo.lock unchanged in terms of resolved versions
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add [workspace.package] license = "MIT" (repository/rust-version SKIPPED: none pre-exist in manifests, no rust-toolchain file).
2. Add [workspace.dependencies]: serde (derive), serde_json, anyhow, clap (derive), chrono = "0.4" base, libc (all identical across >=2 members).
3. Members reference via .workspace = true; whereamid chrono adds features=["serde"]; client chrono base. license.workspace=true in client.
4. Verify cargo build + Cargo.lock resolved versions byte-identical.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Consolidated shared deps into [workspace.dependencies] (serde, serde_json, anyhow, clap, chrono base, libc) + license into [workspace.package]. Members use dep.workspace=true; whereamid layers chrono feature "serde" locally, client uses base chrono. repository/rust-version SKIPPED (none pre-existed). Verified: cargo build OK; Cargo.lock byte-identical; fmt --check clean; just e2e 900 passed / 0 failed (baseline); clippy -D warnings clean. Committed 687229b. Left In Progress for review.

## Review gate (orchestrator, in-thread — objective Cargo.lock oracle) — GO
- The definitive correctness proof for a dep-consolidation is Cargo.lock byte-identity (Cargo\x27s own resolver verifying no member\x27s effective dependency surface moved). CONFIRMED: the commit touches ONLY Cargo.toml files (root + 3 members); Cargo.lock is byte-identical (not in the diff). Zero resolved-version change.
- Spot-checks: whereamid preserves chrono\x27s serde feature (chrono = { workspace = true, features = ["serde"] } layered on the workspace base chrono = "0.4"); clap features identical across members (the task premise "features differ" is STALE); license consolidated (root license = "MIT", whereami-client license.workspace = true). repository/rust-version correctly SKIPPED (none pre-existed — no repo URL, no MSRV/rust-toolchain file; not invented).
- Gate: just e2e 900/900 (unchanged, config-only), clippy 0 -D warnings, fmt clean, cargo build --workspace succeeds.
- Consolidated to [workspace.dependencies]: serde, serde_json, anyhow, clap, chrono(base), libc. Left per-member (divergent/single-member): tokio (full vs test-util), tokio-util, reqwest, rusqlite, toml, tracing, tracing-subscriber, proptest.
- Self-verification is legitimate here: Cargo.lock byte-identity is an objective oracle (like a clean -D warnings compile proving no dead code), stronger than a diff re-read.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Consolidated shared workspace dependencies + license into the root Cargo.toml, eliminating the per-member version duplication that let dep drift go unnoticed.

- [workspace.dependencies]: serde (v1+derive), serde_json (v1), anyhow (v1), clap (v4+derive), chrono (v0.4 base), libc (v0.2) — each identical across its members, so `dep.workspace = true` reproduces the exact surface. chrono\x27s features differ, so the base is in the workspace and whereamid layers `features = ["serde"]` locally. Deps that genuinely diverge or are single-member (tokio full/test-util, reqwest, rusqlite, tracing, ...) were left per-member.
- [workspace.package]: license = "MIT" (was in whereami-client; members now use license.workspace = true). repository/rust-version were NOT invented (none pre-existed; rust-version composes with the not-yet-done task-0164 toolchain pin).

ZERO resolved-version change: Cargo.lock is byte-identical (Cargo\x27s resolver confirms no member\x27s effective dependency surface moved) — the definitive proof for this refactor. cargo build + just e2e (900) + clippy -D warnings all green; no code change. Commit 687229b.
<!-- SECTION:FINAL_SUMMARY:END -->
