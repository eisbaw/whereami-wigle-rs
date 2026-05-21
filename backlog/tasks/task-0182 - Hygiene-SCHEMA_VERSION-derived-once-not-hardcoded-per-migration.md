---
id: TASK-0182
title: 'Hygiene: SCHEMA_VERSION derived once, not hardcoded per migration'
status: To Do
assignee: []
created_date: '2026-05-20 21:37'
labels:
  - hygiene
  - db
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
MPED P3-3. SCHEMA_VERSION = 5 is a const; each migration also hardcodes its target version (params\![4] at db.rs:339, params\![5] at :365). One bump-the-number place would be deriving the target from the function name (e.g. migrate_v3_to_v4 -> target=4) or via a const per-migration.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Each migrate_vN_to_vM function references a const TARGET_VERSION: i64 = M; (so renaming or copy-pasting becomes obviously wrong)
- [ ] #2 OR: a single migrations: &[(from, to, fn)] table at the bottom of db.rs that runs them in order; SCHEMA_VERSION derived as max(to)
- [ ] #3 Pick one; document choice in task notes
- [ ] #4 Existing migration tests pass
<!-- AC:END -->
