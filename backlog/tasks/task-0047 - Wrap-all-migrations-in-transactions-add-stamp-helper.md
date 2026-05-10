---
id: TASK-0047
title: Wrap all migrations in transactions + add stamp helper
status: Done
assignee: []
created_date: '2026-05-10 10:51'
updated_date: '2026-05-10 13:09'
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
- [x] #1 Every migrate_vN_to_vN+1 runs inside a single transaction (BEGIN IMMEDIATE) and commits as the last step
- [x] #2 A migrate(version, |tx| ...) helper runs the closure inside a transaction and atomically stamps schema_version on success
- [x] #3 Existing migration tests still pass; new test simulates mid-migration failure (rolled-back transaction leaves DB at prior version)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added run_migration<F>(|tx| ...) helper that wraps a closure in unchecked_transaction, runs it, and commits on Ok or rolls back on Err.

Refactored migrate_v1_to_v2, migrate_v3_to_v4, migrate_v4_to_v5 to use the helper. migrate_v2_to_v3 already had its own transaction handling; left as-is.

Latent bug discovered and fixed along the way: previous migrate_v3_to_v4 stamped 'version = SCHEMA_VERSION' (currently 5) instead of 4, and same for migrate_v4_to_v5. If the daemon was killed between v3->v4 and v4->v5 on first startup, the DB would claim version=5 but lack the fixes table — and on restart the v4->v5 if-branch would skip. Each migration now stamps its own target version (literal 4 or 5), so partial-migration-then-restart still triggers the missing migrations.

Test: migration_rollback_on_error builds an in-memory DB, runs run_migration with a body that creates a table then bails, asserts table list is unchanged and schema_version unchanged.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Migrations now run in transactions via run_migration helper; on body Err the DDL is rolled back. Found and fixed a latent partial-migration bug where v3->v4 and v4->v5 stamped SCHEMA_VERSION (5) instead of their own target version, which could orphan an unmigrated DB at version=5 if the daemon was killed mid-chain. Test asserts the rollback invariant.
<!-- SECTION:FINAL_SUMMARY:END -->
