---
id: TASK-0030
title: schema_version table should prevent multiple rows
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-09 21:08'
updated_date: '2026-05-09 22:22'
labels: []
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
schema_version table has no constraint preventing multiple rows. get_schema_version uses LIMIT 1 which returns arbitrary row if multiple exist. Add CHECK constraint or use a single-row pattern.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 schema_version table is rebuilt with PRIMARY KEY id INTEGER CHECK (id = 1)
- [x] #2 v2->v3 migration coalesces malformed multi-row tables to MAX(version) atomically (single transaction)
- [x] #3 create_schema_v1 inserts version=1 (not SCHEMA_VERSION) so the migration chain is exercised on fresh DBs
- [x] #4 migrate() is idempotent: running it twice yields exactly one row at SCHEMA_VERSION
- [x] #5 Inserting a second row with id \!= 1 fails the CHECK constraint
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
$1. Bump SCHEMA_VERSION to 3.\n2. Add migrate_v2_to_v3 that rebuilds schema_version with PRIMARY KEY id=1 invariant inside a transaction:\n   - CREATE TABLE schema_version_new (id INTEGER PRIMARY KEY CHECK (id = 1), version INTEGER NOT NULL);\n   - INSERT row with COALESCE(MAX(version), 0) from old table;\n   - DROP old; RENAME new.\n   - All under a single explicit transaction.\n3. Update get_schema_version to read by id=1 (still tolerate the old shape during pre-rebuild reads via a try/fallback).\n4. Update create_schema_v1 to use the new shape going forward (fresh DBs go straight to single-row form), and write (1, SCHEMA_VERSION).\n5. Migration writers (migrate_v1_to_v2, new migrate_v2_to_v3) must UPDATE WHERE id=1.\n6. Test: open in-memory DB, run migrate twice, assert row count == 1 AND version == SCHEMA_VERSION.\n7. Test: hand-build a malformed schema_version with two rows, run migrate, assert it collapses to MAX(version) and a single row.\n8. cargo build/test/clippy/fmt clean.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Bumped SCHEMA_VERSION to 3. Migration v2->v3 rebuilds schema_version transactionally:
  CREATE schema_version_new with id PK + CHECK (id = 1);
  INSERT (1, COALESCE(MAX(version), 0));
  DROP old, RENAME new;
  UPDATE WHERE id=1 AND version < SCHEMA_VERSION (so out-of-order applications cannot downgrade).
Idempotent guard: detect existing CHECK in sqlite_master.sql before rebuild.

Caught and fixed a latent bug while at it: create_schema_v1 used to insert SCHEMA_VERSION (= 3 now) into the freshly-created schema_version table, which would short-circuit migrate_v1_to_v2 / migrate_v2_to_v3 on a brand new DB. Replaced with a literal 1.

Three tests cover the invariant:
- migrate_is_idempotent_and_leaves_single_row: open_memory + 2 more migrate() calls; row count = 1 each time.
- schema_version_rejects_second_row: direct INSERT id=2 fails.
- migrate_collapses_malformed_multirow_schema_version: hand-built v1 table with three rows (1,2,1) gets coalesced to single row at MAX=2 then bumped to SCHEMA_VERSION; subsequent INSERT id=2 fails.

All 35 unit tests + 9 proptests still pass; clippy -D warnings clean; fmt clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
schema_version is now a single-row table

Problem: schema_version had no constraint preventing multiple rows;
get_schema_version did SELECT ... LIMIT 1 which returns an arbitrary row
if more than one exists. A partial migration or two daemon instances
racing on a fresh DB could produce non-deterministic upgrade behaviour.

Fix: SCHEMA_VERSION=3 with a v2->v3 migration that rebuilds the table
transactionally:
  CREATE TABLE schema_version_new (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL
  );
  INSERT (1, COALESCE(MAX(version), 0));
  DROP old; RENAME new;
Then UPDATE the version to SCHEMA_VERSION only when strictly greater (so
an out-of-order higher value cannot be downgraded).

The migration is idempotent: a fast-path check inspects sqlite_master.sql
for the CHECK constraint and skips the rebuild if it is already present.

Also fixed a latent bug uncovered along the way: create_schema_v1 used
to stamp `version = SCHEMA_VERSION` (currently 3) into the v1 table on
fresh DBs, which short-circuited the migration chain. Now writes
`version = 1` and lets migrate() climb the ladder.

Tests:
- migrate_is_idempotent_and_leaves_single_row: triple migrate() on a
  fresh in-memory DB; row count == 1 and version == SCHEMA_VERSION each
  time.
- schema_version_rejects_second_row: direct INSERT (id=2, ...) fails.
- migrate_collapses_malformed_multirow_schema_version: hand-built v1
  schema with three rows (1, 2, 1); migrate() coalesces to a single row
  at MAX(version)=2, then bumps to SCHEMA_VERSION; a follow-up INSERT
  id=2 hits the CHECK.

Verification: cargo build / test (35 unit + 9 proptests pass) /
clippy --all-targets -D warnings / fmt --check all clean.

Schema-version transitions: previous v1 -> now v3 in a single migrate()
call, picking up the 0026 source_priority migration along the way.
<!-- SECTION:FINAL_SUMMARY:END -->
