---
id: TASK-0260
title: >-
  Workspace manifest hygiene: inherit version across crates + publish=false on
  internal crates
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 21:03'
updated_date: '2026-07-08 22:00'
labels:
  - tech-debt
  - tooling
dependencies:
  - TASK-0136
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0136 review (mped-architect, two LOW findings). (1) VERSION triplication: version = "0.6.2" is now hardcoded in three manifests (whereamid, whereami-client, whereami-build-utils) — a single-source-of-truth smell (pre-existing between the two binaries; task-0136 extended it to the new crate). Migrate to workspace-level [workspace.package] version = "0.6.2" + `version.workspace = true` in all member crates so the version lives in ONE place. (2) publish=false: whereami-build-utils (and arguably the two app crates) are internal / not registry-published; add `publish = false` to make that intent explicit and pre-empt a future `cargo publish` choking on an unversioned path build-dep. Both are cheap manifest-only changes; verify `cargo build` + `nix develop -c just e2e` still green (version inheritance can subtly change how cargo resolves; the GIT_REV/version wire tests must still pass). LOW priority, pure hygiene.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Workspace manifest hygiene (commit above). (1) Added [workspace.package] { version = "0.6.2", edition = "2021" } to the root Cargo.toml and switched all three members (whereamid, whereami-client, whereami-build-utils) to version.workspace = true / edition.workspace = true — one source of truth instead of the four-way duplication task-0136 had extended. (2) Added publish = false to all three (internal crates, never published; pre-empts a cargo-publish failure on the unversioned path build-dep whereami-build-utils).

Verified (orchestrator, bounded manifest change): cargo build clean; CARGO_PKG_VERSION still resolves to 0.6.2 (whereami-client --version prints "cli: 0.6.2 (<rev>)"); version_flags 8/8 + version tests + whereamid/client suites green; clippy -D warnings + fmt clean. Dep `version = "1"` lines (serde/clap/etc.) correctly untouched. The wire protocol version constant (task-0145 WIRE_VERSION/SUPPORTED_PROTOCOL_VERSION) is separate and unaffected.
<!-- SECTION:FINAL_SUMMARY:END -->
