---
id: TASK-0047
title: Wrap all migrations in transactions + add stamp helper
status: To Do
assignee: []
created_date: '2026-05-10 10:51'
labels:
  - bug
  - db
  - migrations
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Only migrate_v2_to_v3 wraps in a transaction. v1->v2, v3->v4, v4->v5 each issue CREATE TABLE / ALTER then a separate UPDATE schema_version. A daemon killed mid-migration leaves inconsistent state (table exists with no version stamp, or vice versa). Each migration also rolls its own version-row update with subtly different code; v3->v4 silently depends on v2->v3 having created the single-row shape. Found in v0.4.0 review (mped-architect, keeper-maintainer).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Every migrate_vN_to_vN+1 runs inside a single transaction (BEGIN IMMEDIATE) and commits as the last step
- [ ] #2 A migrate(version, |tx| ...) helper runs the closure inside a transaction and atomically stamps schema_version on success
- [ ] #3 Existing migration tests still pass; new test simulates mid-migration failure (rolled-back transaction leaves DB at prior version)
<!-- AC:END -->
