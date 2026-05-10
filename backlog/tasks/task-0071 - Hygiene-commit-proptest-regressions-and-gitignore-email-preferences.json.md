---
id: TASK-0071
title: 'Hygiene: commit proptest-regressions and gitignore email-preferences.json'
status: To Do
assignee: []
created_date: '2026-05-10 10:59'
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
- [ ] #1 proptests.proptest-regressions is either committed OR explicitly listed in .gitignore (decision documented)
- [ ] #2 backlog/email-preferences.json is gitignored or removed
- [ ] #3 git status -uall in a fresh clone shows zero untracked files
<!-- AC:END -->
