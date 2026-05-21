---
id: TASK-0173
title: Add pre-commit framework / git hooks for just qa
status: To Do
assignee: []
created_date: '2026-05-20 21:35'
labels:
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F13. No .pre-commit-config.yaml, no installed hooks, no lefthook. 'just qa' is convention but nothing enforces it locally.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Simplest: a 'just install-hooks' that drops .git/hooks/pre-commit calling 'just qa'
- [ ] #2 Or heavier: pre-commit-hooks.nix exposing checks.<system>.pre-commit-check from the flake
- [ ] #3 Document opt-in (not auto-enabled; not everyone wants pre-commit-blocked)
- [ ] #4 README mentions the option
<!-- AC:END -->
