---
id: TASK-0212
title: Add result / result-* to .gitignore
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
nix build writes a result symlink into the invocation dir by default; .gitignore covers /target/ and third-party dirs but not result/result-*. Every nix build from the repo root dirties git status and invites accidentally committing a store-path symlink. Ref .gitignore:1-11.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 nix build leaves git status clean
- [ ] #2 result and result-* symlinks cannot be accidentally staged
<!-- AC:END -->
