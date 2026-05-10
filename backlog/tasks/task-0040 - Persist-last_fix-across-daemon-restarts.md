---
id: TASK-0040
title: Persist last_fix across daemon restarts
status: To Do
assignee: []
created_date: '2026-05-10 05:38'
labels:
  - server
  - persistence
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
DaemonState.last_fix is in-memory only; lost on restart. Combined with the cold-start fallback, this means after a restart the daemon must wait for at least one successful resolve before 'where am I' returns anything. Persist last_fix as a single-row last_fix table (mirroring the schema_version invariant from task-0030) and rehydrate on startup. Background resolves should also update the table when they succeed (currently only handle_locate writes last_fix).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 last_fix persisted as a single-row table with CHECK (id = 1)
- [ ] #2 Daemon rehydrates last_fix on startup if table is non-empty
- [ ] #3 resolve_chain successful resolution updates the table even when not invoked from handle_locate
- [ ] #4 Schema migration is forward-only and idempotent
<!-- AC:END -->
