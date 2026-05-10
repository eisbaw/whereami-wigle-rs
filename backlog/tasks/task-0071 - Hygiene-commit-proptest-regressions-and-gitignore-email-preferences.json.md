---
id: TASK-0071
title: 'Hygiene: commit proptest-regressions and gitignore email-preferences.json'
status: Done
assignee: []
created_date: '2026-05-10 10:59'
updated_date: '2026-05-10 14:20'
labels:
  - hygiene
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Two untracked files in the working tree: (1) whereamid/tests/proptests.proptest-regressions — proptest convention is to commit this so seeds are shared across machines; currently in limbo; (2) backlog/email-preferences.json — empty {} from the backlog tooling, never referenced. Found in v0.4.0 review (keeper, Explore).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 proptests.proptest-regressions is either committed OR explicitly listed in .gitignore (decision documented)
- [x] #2 backlog/email-preferences.json is gitignored or removed
- [x] #3 git status -uall in a fresh clone shows zero untracked files
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
proptests.proptest-regressions: kept committed. proptest's convention is to commit this file so failing seeds are shared across machines. The file accidentally got committed during Phase 3; leaving it as-is satisfies the convention.

backlog/email-preferences.json: untracked + added to .gitignore. The file is per-user state from the backlog tooling and should never appear in the repo. Removed from git index (was accidentally committed in Phase 3 via a careless 'git add -A').

git status -uall on a fresh clone now shows zero untracked files matching either pattern.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
proptests.proptest-regressions kept committed (proptest convention). backlog/email-preferences.json untracked + gitignored.
<!-- SECTION:FINAL_SUMMARY:END -->
