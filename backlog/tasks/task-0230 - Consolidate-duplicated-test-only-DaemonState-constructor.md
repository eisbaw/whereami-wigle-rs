---
id: TASK-0230
title: Consolidate duplicated test-only DaemonState constructor
status: To Do
assignee: []
created_date: '2026-07-02 22:12'
labels:
  - robustness
  - tech-debt
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Five hand-written DaemonState struct literals exist for tests (provider.rs, pending.rs, resolver.rs, server.rs dispatch_tests) plus the production one in main.rs. Each new DaemonState field (recently shutdown/tasks/orphan_tasks_aborted from task-0190/0102) must be added in all five places, an error-prone duplication. Extract a single #[cfg(test)] pub(crate) helper (e.g. crate::server::test_daemon_state()) and route the test constructors through it.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A single shared test-only constructor builds DaemonState for tests
- [ ] #2 provider/pending/resolver/server test_state helpers delegate to it
- [ ] #3 Adding a DaemonState field requires editing exactly one test site
<!-- AC:END -->
