---
id: TASK-0182
title: 'Hygiene: SCHEMA_VERSION derived once, not hardcoded per migration'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:37'
updated_date: '2026-07-09 04:06'
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
- [x] #1 Each migrate_vN_to_vM function references a const TARGET_VERSION: i64 = M; (so renaming or copy-pasting becomes obviously wrong)
- [ ] #2 OR: a single migrations: &[(from, to, fn)] table at the bottom of db.rs that runs them in order; SCHEMA_VERSION derived as max(to)
- [x] #3 Pick one; document choice in task notes
- [x] #4 Existing migration tests pass
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add module-level const per migration (V1_TO_V2_TARGET..V8_TO_V9_TARGET) near migrations.
2. Anchor SCHEMA_VERSION = V8_TO_V9_TARGET.
3. Ladder guards (if version < N / version = N) reference the consts.
4. Each migrate fn stamps its own const (params![] / SQL ?1) inside its own existing transaction (atomicity preserved).
5. Add test schema_version_equals_highest_migration_target asserting SCHEMA_VERSION == max(targets).
6. AC#1 chosen over AC#2 table: per-migration consts keep body+stamp in one tx (task-0130 invariant); table-driven loop risks separating stamp from body.
7. Verify: nix develop -c just e2e + cargo fmt --check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented AC#1 (per-migration TARGET consts), commit d854ef3.

- Added V1_TO_V2_TARGET..V8_TO_V9_TARGET module consts near SCHEMA_VERSION.
- Each migrate_vN_to_vM now stamps its OWN const (converted the v1_to_v2 inline `= 2`/`VALUES (2)` and the v2_to_v3 literal 3s to params![CONST]; v3..v9 params![N] -> params![CONST]).
- Ladder guards `if version < N`/`version = N` reference the same consts.
- SCHEMA_VERSION anchored: `const SCHEMA_VERSION: i32 = V8_TO_V9_TARGET;` (stays 9).
- New test schema_version_equals_highest_migration_target asserts SCHEMA_VERSION == max(all target consts).

Approach: chose AC#1 (per-migration consts) over AC#2 (from,to,fn table). Rationale: task-0130 invariant requires each migration body + its version stamp in ONE unchecked_transaction (rollback-on-drop). A table-driven loop tends to separate the stamp from the body; per-migration consts keep the proven atomic pattern with minimal churn and still give a single bump-the-number site per migration. Atomicity preserved and verified (all migration_rollback / rolls_back tests still green).

Note: initially added a module-level MIGRATION_TARGETS slice for the test; clippy -D warnings flagged it dead (only used under #[cfg(test)]). Removed it and inlined the target array in the test instead (the named consts are used by the non-test ladder, so no dead code, no allow()).

ZERO behaviour change: only version NUMBERS became const-sourced; migrations run identically. SCHEMA_VERSION stays 9.

Gate: nix develop -c just e2e PASS (clippy -D warnings clean; cargo fmt --check clean). Workspace 851 tests pass (baseline 849; +1 unique new test, counted twice in the aggregate because whereamid #[cfg(test)] tests compile into both the lib and bin test binaries).

## Review gate (orchestrator, independent) — GO/GO
- QA (2x): 851/851 (+1 unique new test), zero flake; clippy 0 -D warnings; fmt clean.
- Architecture: ATOMICITY PRESERVED (each migrate fn stamps params![CONST] INSIDE its own run_migration/unchecked_transaction, before commit — verified v1_to_v2, v2_to_v3 special-case, v7/v8). Consts wired to BOTH sites (stamp + ladder guard/advance) for every migration, no bare literal left (v2_to_v3 previously had inline 3s — now V2_TO_V3_TARGET at all 3 sites). SCHEMA_VERSION = V8_TO_V9_TARGET (9); the new schema_version_equals_highest_migration_target test builds the array of all target consts, takes max, asserts ==SCHEMA_VERSION (references the real const set, would fail if a higher const were added without bumping). Const values all == prior literals (V1_TO_V2=2..V8_TO_V9=9); zero behavior change.
- Chose AC#1 (per-migration consts) over AC#2 (from,to,fn table) because the table risks separating stamp from body, breaking the task-0130 atomic-transaction invariant; consts preserve it with minimal churn (AC#3 documented).
- LOW (non-blocking, inherent): the anchor-test array is a hand-mirror of the const set (Rust has no compile-time reflection to enumerate consts) — adding a migration must add its const to the array too; acceptable since adding a migration touches this test anyway.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Single-sourced each schema migration\x27s target version so a forgotten bump is compile-visible instead of a silent drift.

Chose AC#1 (per-migration TARGET consts) over AC#2 (from,to,fn table): each migrate_vN_to_vM stamps params![VN_TO_VM_TARGET] inside its own atomic transaction, and the migration ladder\x27s `if version < CONST` guard + `version = CONST` advance reference the SAME const — so a copy-paste that forgets to update the number makes the stamp and guard disagree. The table approach was rejected because it tends to separate the version stamp from the migration body, which would break the task-0130 invariant that body+stamp commit in one transaction (AC#2 is the mutually-exclusive alternative; AC#3 sanctioned the choice).

SCHEMA_VERSION is anchored to V8_TO_V9_TARGET (stays 9); a new test schema_version_equals_highest_migration_target asserts SCHEMA_VERSION == max(all target consts), catching an added migration whose anchor was left behind (and vice-versa).

Zero behavior change: only version numbers became const-sourced; every const equals its prior literal (v2=2..v9=9); migrations run identically; atomicity preserved.

Tests: 851 workspace (+1 new anchor test), dual gate GO/GO, zero flake.

Commit: d986c75 (batched with task-0257).
<!-- SECTION:FINAL_SUMMARY:END -->
