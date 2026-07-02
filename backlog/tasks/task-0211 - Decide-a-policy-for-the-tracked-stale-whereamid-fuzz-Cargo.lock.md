---
id: TASK-0211
title: 'Decide a policy for the tracked, stale whereamid/fuzz/Cargo.lock'
status: To Do
assignee: []
created_date: '2026-07-02 04:53'
labels:
  - nix
  - tooling
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The fuzz lock is committed but pins whereamid at 0.5.0 while the crate is 0.6.2. Because whereamid is a path dep, cargo fuzz run silently rewrites the lock on every invocation (no --locked), dirtying the working tree with a version bump each time. For a detached, dev-only, target-gitignored workspace the file provides little reproducibility value and steady friction. Ref whereamid/fuzz/Cargo.lock, .gitignore:5-7. Distinct from task-0178 (CI fuzz build).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 running the fuzz smoke does not leave the working tree dirty
- [ ] #2 the chosen policy (gitignore vs CI-refresh) is documented in one place
- [ ] #3 if kept, the lock's whereamid version matches the crate version
<!-- AC:END -->
