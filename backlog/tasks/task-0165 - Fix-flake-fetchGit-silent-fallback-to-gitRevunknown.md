---
id: TASK-0165
title: Fix flake fetchGit silent fallback to gitRev='unknown'
status: To Do
assignee: []
created_date: '2026-05-20 21:32'
labels:
  - tooling
  - nix
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F2. nix/package.nix:11-15 uses 'fetchGit { url = ./..; allRefs = true; }'. Under sandboxed nix build with a dirty tree this silently produces gitRev = 'unknown' (the 'or unknown' fallback). build.rs (commit 4f1098a) has its own fallback but it's unclear which path is authoritative.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Drop fetchGit from nix/package.nix entirely
- [ ] #2 Pass self.rev (or self.dirtyRev or 'unknown') from flake.nix into the package as an explicit arg
- [ ] #3 build.rs fallback continues to work for git-archive style consumers
- [ ] #4 Verify: nix build on a clean tree stamps the real git rev; on a dirty tree stamps 'dirty' (not 'unknown')
<!-- AC:END -->
