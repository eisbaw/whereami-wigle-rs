---
id: TASK-0151
title: 'Test: schema migration paths v0/v3/v4 to current'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:29'
updated_date: '2026-07-08 19:35'
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
- [x] #1 Hand-craft a helper that builds a tempfile DB at each historical stamped shape v in {1,2,3,4,5,6} (correct tables + schema_version row + sample aps/not_found/pending/last_fix/fixes rows as that version had them)
- [x] #2 For each historical shape, call Database::open and assert post-migration schema_version == SCHEMA_VERSION (currently 7), every expected current table/index exists, and the seeded rows are preserved/upgraded
- [x] #3 The v3->v7 path is specifically exercised (build a v3-shape DB with sample rows, open, verify migrate_v3_to_v4/v4_to_v5/v5_to_v6/v6_to_v7 all ran and data survived)
- [x] #4 Tests pass in both lib and bin targets; distinct from the existing in-memory step-through (migration_chain_stamps_each_version_in_lockstep) by opening real historical-shape DBs via the public Database::open cascade
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add test helpers in db.rs tests mod:
   - TempDbDir RAII (uses existing unique_tempdir, no new dep; Drop cleans up dir + WAL sidecars)
   - build_historical_shape(path, v): Connection::open(path) -> create_schema_v1 + migrate fns 1..=v (the fns ARE the version definition = authenticity), assert get_schema_version()==v, seed aps/not_found/pending (always) + last_fix (v>=4) + fixes (v>=5)
   - index_exists / assert_close (avoid clippy float_cmp) helpers
2. Test open_each_historical_shape_cascades_to_current: loop v in 1..=6, build file, Database::open, assert v7 + all tables/indexes + single-row schema_version + aps.source_priority/is_mobile + fixes quality cols + seeded rows intact
3. Test open_v3_shape_runs_all_upper_migrations_and_preserves_rows (AC#3): pre-open assert v3 lacks last_fix/fixes/is_mobile/quality; open; assert all present + rows survived
4. Gate: nix develop -c cargo test -p whereamid (lib+bin), just e2e, clippy -D warnings, fmt --check; run new tests twice
5. Commit code only, no attribution, conventional commit
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented file-backed migration-cascade tests in whereamid/src/db.rs tests mod (commit 2ef8e74).
- Helpers: TempDbDir (RAII, reuses unique_tempdir; NO new dep), build_historical_shape (create_schema_v1 + migrate fns 1..=v = authentic shape), seed_sample_rows, assert_current_shape_and_seed, index_exists, assert_close (avoids clippy float_cmp).
- Tests: open_each_historical_shape_cascades_to_current (v1..=6), open_v3_shape_runs_all_upper_migrations_and_preserves_rows (AC#3 v3->v7).
- Authenticity: each vN shape built ONLY from real migrate fns; verified tests FAIL (aps.is_mobile missing) when migrate_v5_to_v6 stubbed to no-op, then reverted.
Gate: cargo test -p whereamid 203 (lib) + 461 (bin) + 12 + 4 pass; new tests run twice green in BOTH targets (6 lib / 7 bin filtered). clippy --all-targets -D warnings EXIT=0. cargo fmt --check clean. just e2e EXIT=0.
Left In Progress for mandatory review gate; ACs not checked.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added file-based migration-cascade coverage: for each historical stamped shape v in {1..6}, build a tempfile DB at exactly that shape (create_schema_v1 + real migrate fns up to v, then seed sample aps/not_found/pending/last_fix(v>=4)/fixes(v>=5) rows), then Database::open() to run the PUBLIC migrate() cascade to v7 and assert schema_version==7, all current tables/indexes exist, and seeded rows survived by value. Distinct from the pre-existing in-memory step-through (migration_chain_stamps_each_version_in_lockstep) which never opens a real file.

Changes (commit 2ef8e74, whereamid/src/db.rs, TEST-ONLY +393 in #[cfg(test)]): build_historical_shape helper, TempDbDir RAII (cleans the dir incl. -wal/-shm sidecars on panic, via the crate's existing unique_tempdir — no new dep), open_each_historical_shape_cascades_to_current, open_v3_shape_runs_all_upper_migrations_and_preserves_rows.

AUTHENTICITY: each vN shape is built ONLY from create_schema_v1 + the real migrate_v*_to_v* fns (those fns ARE the version definition) + a stamp==v assertion before seeding, so no later column/table can leak into an earlier shape. BITE: each of the 6 migrations has a dedicated post-cascade assertion reached through open() by a low origin that does not trigger shape-repair (repair keys off the stored stamp, capped at v, so it declines on a genuine vN and the real cascade must build every later shape — verified against migrate()/highest_satisfied_shape_version). Implementer empirically mutation-tested migrate_v5_to_v6 (stub->no-op -> tests FAIL -> reverted); mped independently confirmed the other five bite by static inspection.

Verification: review mped-architect GO (traced all six migrations + repair path; no correctness defects; findings all LOW/optional) + orchestrator gate re-run (historical_shape tests pass in BOTH lib and bin targets x2, clippy -D warnings clean, fmt clean, db lib 203). 

Known limitations (honest, not defects): (1) authenticity is "today's migrate chain's definition of vN", not archived historical DDL — guards against future re-ordering/breakage of the current chain, cannot catch a divergence where older daemon code wrote a subtly different on-disk vN (inherent; crate archives no historical DDL). (2) source_priority backfill VALUE is not asserted per-origin (for v>=2 the row seeds after the backfill ran on an empty table -> default 0; the backfill value is owned by migrate_v1_to_v2_*priorit* tests). Optional test-strengthening (not_found value assertion + per-migration mutants) filed as task-0257.
<!-- SECTION:FINAL_SUMMARY:END -->
