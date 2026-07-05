---
id: TASK-0211
title: 'Decide a policy for the tracked, stale whereamid/fuzz/Cargo.lock'
status: Done
assignee: []
created_date: '2026-07-02 04:53'
updated_date: '2026-07-05 09:05'
labels:
  - nix
  - tooling
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The tracked lock for the detached cargo-fuzz workspace pins the whereamid path dependency at 0.4.0 while the crate is 0.6.2. Because cargo fuzz runs without --locked, it rewrites the lock during ordinary fuzzing and dirties the worktree. The separately built fuzz workspace already treats target, corpus, and artifacts as generated state, so tracking this stale lock adds friction without current reproducibility enforcement. References: whereamid/fuzz/Cargo.lock and .gitignore. Distinct from TASK-0178 (CI fuzz build); if future CI requires --locked reproducibility, that task must intentionally revisit this policy.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 running the fuzz smoke does not leave the working tree dirty
- [x] #2 the chosen policy (gitignore vs CI-refresh) is documented in one place
- [x] #3 if kept, the lock's whereamid version matches the crate version
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Treat the detached cargo-fuzz lock as generated state: remove it from version control and ignore it with a durable explanatory policy comment.
2. Regenerate the ignored lock by running the full fuzz smoke and prove git status is unchanged apart from the intended task diff.
3. Run exact just e2e and focused ignore/status checks.
4. Obtain parallel QA and architecture reviews, resolve findings, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
First review gate: QA GO; architecture NO-GO on wording only. Correct the observed baseline version from 0.5.0 to 0.4.0, remove fragile line citations, and avoid calling the workspace development-only because TASK-0178 plans future CI use.

Corrective review gate: GO from qa-test-runner and mped-architect. After staging the intended deletion and policy, exact `just fuzz` passed all five 15-second targets. Pre/post porcelain status was byte-identical and the staged diff hash remained 6ed947bbaf6fbe80a3d40cd1be273aac878af956b2b2d28a335f299467a6b6ce; regenerated ignored lock contained whereamid 0.6.2. AC3 is not applicable because the lock is not kept.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Adopted a generated/untracked policy for the detached cargo-fuzz lock, documented it in .gitignore, and removed the stale tracked lock (which pinned whereamid 0.4.0). The full five-target fuzz smoke regenerated an ignored 0.6.2 lock without changing staged status or diff hash. Exact E2E passed 495 tests with strict clippy; mandatory QA and architecture reviews returned GO.
<!-- SECTION:FINAL_SUMMARY:END -->
