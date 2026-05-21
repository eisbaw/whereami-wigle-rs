---
id: TASK-0151
title: 'Test: schema migration paths v0/v3/v4 to current'
status: To Do
assignee: []
created_date: '2026-05-20 21:29'
labels:
  - tests
  - migrations
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P0-4. db.rs:1134 migrate_v1_to_v2_backfills_source_priority only tests v1->v2 backfill values. open_memory always runs the whole cascade from v0; calling migrate() twice on v0 is NOT the same as opening a v4 DB and discovering v5 runs cleanly. Re-ordering migrations would still pass the fresh-DB test.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Hand-craft an SQL script per historical version v ∈ {0, 1, 2, 3, 4}
- [ ] #2 For each: create a tempfile DB matching that exact v-shape (all tables, correct schema_version row, sample data), then call Database::open
- [ ] #3 Assert post-migration: schema_version=5, every expected table+index exists, seed data preserved
- [ ] #4 v3->v5 path specifically tested: build a v3 DB with sample aps/not_found/pending rows; open; verify migrate_v3_to_v4 and migrate_v4_to_v5 ran
<!-- AC:END -->
