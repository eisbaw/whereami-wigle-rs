---
id: TASK-0147
title: Consolidate two test_state helpers
status: To Do
assignee: []
created_date: '2026-05-20 21:28'
labels:
  - hygiene
  - tests
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/resolver.rs:466 + src/pending.rs:140 define near-identical test_state helpers. Same body modulo a comment. Pattern same as task-0059 (consolidated From/TryFrom impls).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Extract a single crate-local test helper (e.g. crate::tests::test_state or whereamid/tests/common.rs)
- [ ] #2 Both resolver.rs and pending.rs tests use it
- [ ] #3 All existing tests pass
<!-- AC:END -->
