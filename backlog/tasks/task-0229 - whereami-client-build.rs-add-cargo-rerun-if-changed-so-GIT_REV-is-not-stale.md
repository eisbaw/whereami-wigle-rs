---
id: TASK-0229
title: 'whereami-client build.rs: add cargo:rerun-if-changed so GIT_REV is not stale'
status: To Do
assignee: []
created_date: '2026-07-02 17:46'
updated_date: '2026-07-02 17:47'
labels:
  - cli
  - build
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
QA review of the 0183/0198/0199 cycle found the embedded GIT_REV goes stale: NEITHER whereami-client/build.rs NOR whereamid/build.rs emits cargo:rerun-if-changed, so commits that do not touch that crate's sources leave the old rev baked into the binary (observed: client banner said 71ba621 while HEAD was 5b59f87). Cosmetic (version banner / debug header only) but misleading when debugging cli/daemon skew. Standard fix: emit rerun-if-changed for .git/HEAD and the ref file it points at, in both build scripts.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 After a new commit, a rebuilt whereami binary reports the current HEAD rev in whereami version
- [ ] #2 build.rs emits cargo:rerun-if-changed for .git/HEAD and the current branch ref so incremental builds pick up new commits
- [ ] #3 whereamid's build.rs gets the same fix (both crates embed GIT_REV)
<!-- AC:END -->
