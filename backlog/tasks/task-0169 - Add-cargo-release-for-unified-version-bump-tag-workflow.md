---
id: TASK-0169
title: Add cargo-release for unified version-bump + tag workflow
status: To Do
assignee: []
created_date: '2026-05-20 21:34'
labels:
  - tooling
  - release
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F7. whereami-client is publish-ready (description/license/readme set in Cargo.toml). No just publish recipe, no cargo-release config. Release process today is manual: edit two Cargo.toml versions, edit Cargo.lock, edit CHANGELOG, commit, tag. Three places to forget.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add pkgs.cargo-release to devShell
- [ ] #2 Add [workspace.metadata.release] with shared-version = true
- [ ] #3 Justfile: 'just release LEVEL' (LEVEL = patch | minor | major) runs cargo release {{LEVEL}} --execute
- [ ] #4 GIT_REV file written by Nix preBuild is honored by cargo publish path (test by 'cargo publish --dry-run' from clean state)
- [ ] #5 Optional: cargo release also pushes the tag to remote (or document that as a manual step)
<!-- AC:END -->
