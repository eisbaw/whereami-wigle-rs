---
id: TASK-0136
title: Deduplicate byte-identical build.rs across whereamid and whereami-client
status: To Do
assignee: []
created_date: '2026-05-20 21:22'
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
- [ ] #1 Option A: extract a whereami-build-utils crate exposing fn emit_git_rev_env(); both build.rs files become 'use whereami_build_utils::emit_git_rev_env; fn main() { emit_git_rev_env(); }'
- [ ] #2 Option B: shared via 'include\!("../whereamid/build.rs")' from the client
- [ ] #3 Verify: nix build produces a binary with GIT_REV set; cargo build does too (via the in-tree git command)
- [ ] #4 Doc-comment on the shared helper: explains the Nix preBuild path vs the git command path; either is authoritative
<!-- AC:END -->
