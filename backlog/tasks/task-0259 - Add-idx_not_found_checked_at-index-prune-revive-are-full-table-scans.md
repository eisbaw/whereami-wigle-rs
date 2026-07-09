---
id: TASK-0259
title: Add idx_not_found_checked_at index (prune + revive are full-table scans)
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 20:08'
updated_date: '2026-07-09 06:03'
labels:
  - performance
  - db
  - tech-debt
dependencies:
  - TASK-0110
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0110 review (mped-architect, MEDIUM). There is NO index on not_found.checked_at (confirmed indexes: idx_fixes_at, idx_aps_geo, idx_aps_last_seen). So BOTH prune_not_found (DELETE ... WHERE checked_at < ?, every drain pass, task-0110) AND revive_expired_not_found (SELECT ... WHERE checked_at <= ? LIMIT 5, every drain pass) are FULL-TABLE SCANS on the exact table task-0110 says grows huge — and they run under the single-writer DbHandle.write lock, so a long scan blocks record_not_found/set_last_fix/pending writes in the writer FIFO. Inconsistent with precedent: fixes/aps got expiry/range indexes AND query-plan assertion tests (db.rs ~3684/3742 "must SEARCH idx_..."). Fix: add a schema migration v7->v8 creating idx_not_found_checked_at, and a query-plan assertion test (EXPLAIN QUERY PLAN shows SEARCH via the index for both the prune DELETE and the revive SELECT). Makes both O(log N + matches) instead of O(N)/pass. Mirror the existing idx_fixes_at migration + query-plan test pattern.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Schema migration v9->v10 (SCHEMA_VERSION 9->10, via a V9_TO_V10_TARGET const per task-0182) creates idx_not_found_checked_at ON not_found(checked_at), inside run_migration atomically with the version stamp; the historical create_schema_v1 is NOT edited (fresh DBs get the index via the cascade, per the task-0123 lesson); cascade tests extended to v10
- [x] #2 A query-plan assertion test: EXPLAIN QUERY PLAN for BOTH the prune_not_found DELETE and the revive_expired_not_found SELECT shows SEARCH USING idx_not_found_checked_at (not SCAN), mirroring the existing idx_fixes_at query-plan test
- [x] #3 Existing not_found/migration/cascade tests pass; no behavior change beyond the index + faster query plan (prune/revive results identical)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add V9_TO_V10_TARGET=10 const; SCHEMA_VERSION=V9_TO_V10_TARGET.
2. Add migrate_v9_to_v10 (CREATE INDEX IF NOT EXISTS idx_not_found_checked_at ON not_found(checked_at) + version stamp in one run_migration tx), mirroring migrate_v8_to_v9.
3. Register in migrate() ladder after v8->v9 block.
4. Extend schema_version_equals_highest_migration_target target list with V9_TO_V10_TARGET.
5. Extend cascade infra: build_historical_shape 1..=9 (add v8->v9 step), open_each_historical_shape loop 1..=9, add idx_not_found_checked_at to assert_current_shape_and_seed index list, extend lockstep test with v9->v10 step.
6. Add open_v9_shape_creates_not_found_index_at_v10 test (index present + not_found row survives).
7. AC#2: query-plan test EXPLAIN QUERY PLAN for prune DELETE and revive SELECT asserts SEARCH via idx_not_found_checked_at, not SCAN.
8. nix develop -c just e2e + cargo fmt --check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented v9->v10 migration adding idx_not_found_checked_at ON not_found(checked_at) (commit 6731425).

- V9_TO_V10_TARGET=10 const added; SCHEMA_VERSION=V9_TO_V10_TARGET=10; ladder + schema_version_equals_highest_migration_target list extended. Anchor test passes.
- migrate_v9_to_v10: CREATE INDEX IF NOT EXISTS + version stamp in ONE run_migration tx (mirrors migrate_v8_to_v9). Idempotent; a v9 DB opens at v10, a fresh DB cascades. create_schema_v1 NOT edited.
- AC#2 query-plan test not_found_prune_and_revive_use_index_not_full_scan: EXPLAIN QUERY PLAN using the verbatim prune DELETE and revive SELECT SQL both show SEARCH not_found USING INDEX idx_not_found_checked_at (asserts SEARCH + index name, forbids SCAN not_found).
- Cascade tests extended to v10: build_historical_shape 1..=9 (+v8->v9 step), open_each_historical_shape loop 1..=9, lockstep steps v9->v10, idx_not_found_checked_at added to verifier index list, new open_v9_shape_creates_not_found_index_at_v10 (PRAGMA index_list + row/value survives).
- seed_sample_rows made channel-aware: v9 dropped aps/pending.channel (task-0123), so seeding a genuine v9 shape must omit channel; pre-v9 still seeds channel=6/11 for the v8 drop test.
- No prune/revive LOGIC changed; only the index/plan.

Gate: nix develop -c just e2e -> 880 passed / 0 failed (baseline 876; +4 = 2 new #[test] fns x lib+bin targets, db.rs compiles into both). clippy -D warnings clean. cargo fmt --check clean. Left In Progress for review.

## Review gate (orchestrator, independent) — GO/GO
- QA (2x): 880/880, zero flake; clippy 0 -D warnings; fmt clean. Query-plan test + v9->v10 cascade + anchor(=10) + v8->v9 drop (after seed_sample_rows change) all pass.
- Architecture: all 4 CONFIRMED. (1) Index GENUINELY serves the real queries — production prune DELETE (db.rs:3282 `WHERE checked_at < ?1`) and revive SELECT (3553 `WHERE checked_at <= ?1 LIMIT ?2`) are byte-identical to the test SQL; bare column, no function wrap -> index-usable; EXPLAIN asserts SEARCH USING idx_not_found_checked_at AND !SCAN. Test runs on open_memory (v0->v10 cascade) so it also proves the fresh-DB path builds the index. (2) Migration atomic (CREATE INDEX IF NOT EXISTS + stamp in one run_migration tx, mirrors v8->v9), idempotent, SCHEMA_VERSION=10, anchor test includes V9_TO_V10_TARGET, create_schema_v1 untouched; cascade test builds real v9 (index absent) -> v10 (present via sqlite_master + PRAGMA index_list, row survives). (3) seed_sample_rows channel-aware SOUND: has_channel=version<9; pre-v9 INSERT byte-identical (channel=6 still seeded -> v8->v9 drop test intact), v9+ omits channel (builds cleanly); frequency seeded in both branches; no pre-existing test built a v9 shape so nothing altered. (4) prune/revive logic bytes untouched.
- LOW (accepted, consistent with the idx_fixes_at precedent): the plan-test copies the two WHERE fragments as string literals (sync-comment present, not a shared const); the negative assert keys on the "SCAN not_found" substring. Both cosmetic; mirror the existing idx_fixes_at test pattern.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Indexed not_found.checked_at so the per-drain-pass prune (DELETE WHERE checked_at < cutoff) and revive (SELECT WHERE checked_at <= cutoff LIMIT n) stop being O(N) full-table scans under the single-writer lock — they blocked other writes on the exact table task-0110 says grows large.

Migration v9->v10 (SCHEMA_VERSION 9->10 via a V9_TO_V10_TARGET const, task-0182 pattern): CREATE INDEX IF NOT EXISTS idx_not_found_checked_at ON not_found(checked_at), atomic (index + version stamp in one run_migration transaction, mirroring v8->v9), idempotent. create_schema_v1 untouched (fresh DBs get the index via the v1->v10 cascade, task-0123 lesson).

Tests: a query-plan assertion (EXPLAIN QUERY PLAN shows SEARCH USING idx_not_found_checked_at, not SCAN, for BOTH the real prune DELETE and revive SELECT — SQL copied verbatim from production, mirroring the idx_fixes_at precedent); a v9->v10 cascade test (index absent at v9, present at v10, not_found row survives); the anchor test now reflects 10. Also made the test helper seed_sample_rows channel-aware (channel column exists v1-v8, dropped at v9) so a genuine v9 shape can be built — pre-v9 seeding is byte-identical so the v8->v9 drop test is unaffected.

No behavior change: prune/revive logic untouched; same rows deleted/revived, only the query plan changes. 880 workspace tests, dual gate GO/GO. Commit 6731425.
<!-- SECTION:FINAL_SUMMARY:END -->
