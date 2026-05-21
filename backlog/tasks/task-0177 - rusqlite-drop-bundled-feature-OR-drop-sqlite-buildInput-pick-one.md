---
id: TASK-0177
title: 'rusqlite: drop bundled feature OR drop sqlite buildInput (pick one)'
status: To Do
assignee: []
created_date: '2026-05-20 21:35'
labels:
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F19. whereamid/Cargo.toml:17 has rusqlite features=['bundled'] (compiles SQLite from source). nix/package.nix:21 adds 'sqlite' buildInput. Bundled doesn't link the system library, so the buildInput is dead weight. Either drop bundled (faster builds, smaller binary, security updates via OS) or drop the sqlite buildInput.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Decision recorded in task notes
- [ ] #2 If dropping bundled: rusqlite features become e.g. ['backup','blob','functions'] (preserve any used features); pkg-config picks up system sqlite via the buildInput
- [ ] #3 If dropping buildInput: remove 'sqlite' from buildInputs in nix/package.nix
- [ ] #4 nix build still produces a working binary; cargo test still passes
- [ ] #5 Binary size delta measured + recorded in notes
<!-- AC:END -->
