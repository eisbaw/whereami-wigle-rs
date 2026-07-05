---
id: TASK-0212
title: Add result / result-* to .gitignore
status: Done
assignee: []
created_date: '2026-07-02 04:53'
updated_date: '2026-07-05 07:45'
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
- [x] #1 nix build leaves git status clean
- [x] #2 result and result-* symlinks cannot be accidentally staged
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add repository-root ignore rules for the default Nix result symlink and named result-* output links, without hiding similarly named paths in nested source directories. 2. Build a cached flake check with the default result link and a named result-* link, then verify both are ignored, absent from git status, and rejected by ordinary git add; remove the generated links after verification. 3. Run nix-shell just e2e, require parallel qa-test-runner and mped-architect GO, complete the tracker, rerun E2E, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Preimplementation contract (2026-07-05): use anchored `/result` and `/result-*` patterns. Acceptance means normal status/add workflows ignore root Nix output symlinks; force-add remains intentionally possible in Git and is not a realistic accidental path. Verify with an actual default nix build plus a named out-link, git check-ignore, status comparison, and ordinary add behavior. Do not add a broad unanchored rule that could hide nested project files.

Final review verification (2026-07-05): real default and named Nix output links were ignored, absent from full untracked status, and rejected by ordinary add without index mutation; nested and near-match controls remained visible. Parallel QA and architecture returned GO with no findings on frozen hash 39562fac.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added repository-root `/result` and `/result-*` ignore rules for Nix output links. Actual cached Nix builds proved default and named links remain out of status and ordinary staging, while nested result paths and near-match names remain visible. Generated links were removed after testing. Verified 474-test E2E with strict Clippy and final parallel QA/architecture GO with no findings; intentional `git add -f` remains the standard explicit Git override.
<!-- SECTION:FINAL_SUMMARY:END -->
