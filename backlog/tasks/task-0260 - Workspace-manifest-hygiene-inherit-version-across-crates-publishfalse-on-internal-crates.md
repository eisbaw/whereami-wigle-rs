---
id: TASK-0260
title: >-
  Workspace manifest hygiene: inherit version across crates + publish=false on
  internal crates
status: To Do
assignee: []
created_date: '2026-07-08 21:03'
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
