---
id: TASK-0166
title: Add .envrc + direnv integration for one-step shell
status: To Do
assignee: []
created_date: '2026-05-20 21:32'
labels:
  - tooling
  - onboarding
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F4. Justfile says 'recipes assume you are inside the dev shell: nix develop'. New contributors need two steps (nix develop + just). No direnv integration. Onboarding goal of one command not met.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add .envrc with single line: use flake
- [ ] #2 Add .envrc.recommended or doc in README: 'direnv allow' is the only setup step needed
- [ ] #3 Optional belt-and-suspenders: a private justfile recipe [private] _in-shell: that re-execs under 'nix develop -c' if IN_NIX_SHELL is unset, so 'just build' works outside the shell too
- [ ] #4 README updated with the one-step onboarding
<!-- AC:END -->
