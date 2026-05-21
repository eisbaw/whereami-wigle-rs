---
id: TASK-0111
title: Migration v1->v2 hardcoded priorities consistency test
status: To Do
assignee: []
created_date: '2026-05-20 21:12'
labels:
  - correctness
  - migrations
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/db.rs:225-232. The v1->v2 backfill hardcodes source priority numbers in SQL, with a comment 'do not call Source::priority() here'. This is correct discipline (migrations are frozen archaeology, not live code). But there's no test asserting the hardcoded migration numbers EQUAL Source::priority() AT THE TIME OF MERGE — they have to be equal until/unless a future change deliberately diverges. If Apple is renumbered to 50 in the enum, the migration would silently set old rows to the old number with no signal.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Unit test: run migrate_v1_to_v2 on a fresh DB with one row per Source string variant, then assert each row's source_priority equals Source::from_str(s).unwrap().priority()
- [ ] #2 Doc-comment on migrate_v1_to_v2 explicitly says: 'these constants must equal Source::priority() AT THE TIME THIS MIGRATION SHIPPED; later renumbering of the enum requires a new migration, not editing this one'
<!-- AC:END -->
