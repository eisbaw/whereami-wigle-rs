---
id: TASK-0229
title: 'whereami-client build.rs: add cargo:rerun-if-changed so GIT_REV is not stale'
status: Done
assignee: []
created_date: '2026-07-02 17:46'
updated_date: '2026-07-05 10:09'
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
- [x] #1 After a new commit, a rebuilt whereami binary reports the current HEAD rev in whereami version
- [x] #2 build.rs emits cargo:rerun-if-changed for .git/HEAD and the current branch ref so incremental builds pick up new commits
- [x] #3 whereamid's build.rs gets the same fix (both crates embed GIT_REV)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. In both build scripts, emit rerun-if-changed for the actual Git HEAD path and current symbolic branch ref using git rev-parse --git-path.
2. When the loose branch ref is absent (packed/unborn), avoid a permanently missing watch: monitor the nearest existing refs directory for loose-ref creation and the existing packed-refs file for packed updates.
3. Preserve detached-HEAD and clean Nix source fallbacks without changing GIT_REV selection.
4. In a temporary clone, prove attached loose-ref refresh, packed-ref no-op freshness, one rebuild/update after an empty commit, and freshness again afterward for both packages and the client banner.
5. Run exact just e2e, parallel QA and architecture reviews, resolve findings, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carry from task-0209: the nix GIT_REV SOURCE changed. package.nix no longer uses builtins.fetchGit (it aborts under `nix flake check` pure-eval — the flake source is a non-git store path). flake.nix now threads self.shortRev / self.dirtyShortRev / "unknown" into preBuild, which writes whereamid/GIT_REV + whereami-client/GIT_REV. build.rs is UNCHANGED and still tries `git rev-parse --short HEAD` first, then falls back to the GIT_REV file. This task (rerun-if-changed) is still needed and orthogonal: in a nix build git rev-parse fails (no .git in cleanCargoSource) so the file wins; in a plain `cargo build` there is no rerun-if-changed on .git/HEAD, so a rebuilt binary can carry a stale rev. Observed on a dirty tree the embedded rev is now e.g. `aa014bf-dirty` (dirtyShortRev).

First review gate: both qa-test-runner and mped-architect returned NO-GO. A packed current branch has no loose ref file; emitting that missing path makes Cargo report both packages Dirty and rebuild forever. Required correction: emit only existing files, watch an existing refs directory plus packed-refs while loose ref is absent, and prove packed no-op -> empty-commit refresh -> no-op fresh.

Corrective review gate: qa-test-runner and mped-architect both GO on frozen diff 83ca813eaae0413a186da0473769cb2edc309560710dc597697050deeb3682b3. Independent QA reproduced packed current ref -> unchanged Fresh, empty commit -> exactly one rebuild of both packages with new GIT_REV/client banner, then unchanged Fresh. No missing watched paths were emitted; linked worktree, unborn, detached, and non-Git cases were also checked.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Both build scripts now track worktree-safe Git metadata so incremental builds refresh embedded revisions after commits. Existing loose refs are watched exactly; packed/unborn refs watch an existing refs directory plus packed-refs until a loose ref appears, avoiding permanent Cargo Dirty rebuilds. Detached HEAD and Nix GIT_REV fallback behavior remain unchanged. Attached, packed, linked-worktree, detached, and non-Git integration checks passed; E2E, QA, and architecture reviews returned GO.
<!-- SECTION:FINAL_SUMMARY:END -->
