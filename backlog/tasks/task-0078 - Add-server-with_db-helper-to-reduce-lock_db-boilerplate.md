---
id: TASK-0078
title: 'Add server::with_db helper to reduce lock_db boilerplate'
status: To Do
assignee: []
created_date: '2026-05-10 11:02'
labels:
  - refactor
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
lock_db is called 30+ times. The pattern is almost always 'let db = lock_db(state); let x = db.foo().unwrap_or(default); drop(db);' in a single statement. A state.with_db(|db| ...) helper that takes a closure and returns the result would shrink call sites and centralize the poison-recovery + lock-scope behavior. Don't delete lock_db; add the wrapper. Found in v0.4.0 review (swe-gardener).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 DaemonState::with_db<R>(&self, f: impl FnOnce(&Database) -> R) -> R helper exists
- [ ] #2 At least handle_stats and handle_debug are converted as proof-of-pattern
- [ ] #3 lock_db remains for sites that hold the guard across multiple calls (resolver chain)
<!-- AC:END -->
