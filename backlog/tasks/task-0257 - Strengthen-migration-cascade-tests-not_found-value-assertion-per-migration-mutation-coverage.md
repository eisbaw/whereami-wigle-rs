---
id: TASK-0257
title: >-
  Strengthen migration-cascade tests: not_found value assertion + per-migration
  mutation coverage
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 19:34'
updated_date: '2026-07-09 04:06'
labels:
  - tests
  - tech-debt
dependencies:
  - TASK-0151
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Optional follow-up from task-0151 review (mped-architect, both LOW). The historical-shape cascade tests (task-0151, db.rs ~5071-5470) are solid; two optional hardenings: (A) the not_found seeded-row survival check asserts only COUNT(*)==1 by bssid, not column VALUES — a migration that rebuilt not_found preserving the PK but corrupting first_seen/last_seen/checked_at would pass (low risk: no migration currently touches not_found). Query back one timestamp column for symmetry with the aps/pending/last_fix/fixes checks. (B) The "genuinely bites" property was EMPIRICALLY mutation-tested for only migrate_v5_to_v6 (stub->no-op->tests fail); v2_to_v3/v3_to_v4/v4_to_v5/v6_to_v7 bite is confirmed by static inspection (each has a dedicated post-cascade assertion) but not measured. Optionally add a single mutant check per remaining migration to make the guarantee measured rather than argued. Neither blocks; both are test-strengthening only.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The not_found seeded-row survival check in the historical-shape cascade tests asserts a column VALUE (query back a timestamp, e.g. checked_at or first_seen), not only COUNT(*)==1 — symmetric with the aps/pending/last_fix/fixes value checks
- [x] #2 The 'migration genuinely bites' property is verified (one-time mutation check: stub each of v2_to_v3/v3_to_v4/v4_to_v5/v6_to_v7/v7_to_v8/v8_to_v9 to a no-op, confirm a cascade test fails, revert) and the result reported in task notes — measured, not just argued
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. AC#1: add SEED_NOT_FOUND_CHECKED_AT const; use it in seed_sample_rows not_found INSERT; assert checked_at value survives cascade in assert_current_shape_and_seed (symmetric with aps/pending/last_fix/fixes value checks).
2. AC#2 (report-only): temporarily stub each of v2_to_v3,v3_to_v4,v4_to_v5,v6_to_v7,v7_to_v8,v8_to_v9 body to no-op (keep version stamp), run cascade tests, confirm >=1 fails (bites), revert. Report per-migration PASS/bite in notes. Do NOT commit stubs.
3. Verify gate.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented AC#1 + measured AC#2, commit d854ef3.

AC#1 (not_found value assertion): added SEED_NOT_FOUND_CHECKED_AT const; seed_sample_rows now inserts checked_at via ?2 param; assert_current_shape_and_seed queries `SELECT COUNT(*), MAX(checked_at)` and asserts the value equals SEED_NOT_FOUND_CHECKED_AT (symmetric with aps/pending/last_fix/fixes value checks). Was COUNT(*)==1 only.

AC#2 (mutation-bite, REPORT-ONLY — no stub committed): for each migration I temporarily stubbed the body to a no-op (kept the version stamp), ran the full db::tests module, recorded failures, then reverted. Baseline = 96 db tests green. Results (all BITE):
- migrate_v2_to_v3 (skip single-row rebuild): 90/96 FAIL. Whole suite collapses because every DB open runs the ladder and the id=1 stamp errors on the un-rebuilt multi-row table. Cascade tests fail. BITES (hard).
- migrate_v3_to_v4 (skip CREATE last_fix): 22 FAIL incl open_each_historical_shape_cascades_to_current, open_v3_shape_*. BITES.
- migrate_v4_to_v5 (skip CREATE fixes): 87 FAIL incl the cascade tests. BITES (fixes table underpins much).
- migrate_v6_to_v7 (skip ALTER quality cols): 23 FAIL incl open_each_historical_shape_cascades_to_current, open_v3_shape_*. BITES.
- migrate_v7_to_v8 (skip key rename): 3 FAIL: migration_chain_stamps_each_version_in_lockstep, open_v7_shape_renames_wigle_counter_key_losslessly, open_v7_shape_with_both_counter_keys_fails_loud_not_silent. BITES. NOTE: the GENERIC open_each_historical_shape test does NOT catch this (it seeds no metadata counter) — the dedicated v7 cascade tests do. Coverage is adequate; no gap.
- migrate_v8_to_v9 (skip DROP channel): 4 FAIL incl open_each_historical_shape_cascades_to_current, open_v3_shape_*, open_v8_shape_drops_channel_column_at_v9. BITES.

All six BITE. Combined with the prior-known v1_to_v2/v5_to_v6, every migration is now measured (not argued) to be detected by at least one post-cascade assertion. No coverage gap found; no extra assertion needed. Stubs were reverted and the tree re-verified green before commit.

Gate: nix develop -c just e2e PASS; cargo fmt --check clean.

## Review gate (orchestrator, independent) — GO/GO
- AC#1 (value assertion): SEED_NOT_FOUND_CHECKED_AT="2020-02-02T00:00:00Z" seeded into not_found.checked_at BEFORE the cascade; post-cascade the test queries checked_at back and asserts it survived verbatim — reviewer confirmed non-tautological (survival of a pre-seeded value, symmetric with aps/pending/last_fix/fixes).
- AC#2 (mutation-bite, measured): the 6 remaining migrations (v2_to_v3..v8_to_v9; v1_to_v2/v5_to_v6 already confirmed prior) were each stubbed to no-op (stamp kept), cascade tests re-run: ALL BITE (v2_to_v3 90 fail, v3_to_v4 22, v4_to_v5 87, v6_to_v7 23, v7_to_v8 3 via the dedicated wigle-counter test, v8_to_v9 4 via the drop-channel test), stubs reverted, nothing committed. Reviewer independently confirmed every migration has a post-cascade assert that fails if it were a no-op — NO coverage gap. (LOW/pre-existing: v1_to_v2/v5_to_v6 bite via column_exists only, so a column-added-but-backfill-skipped variant would slip — noted, not introduced here.)
- Value assertion + anchor test all green (851 workspace, zero flake).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Strengthened the historical-shape migration-cascade tests (follow-up from the task-0151 review).

(A) not_found survival now asserts a column VALUE: a known checked_at timestamp is seeded pre-cascade and asserted to survive verbatim, symmetric with the existing aps/pending/last_fix/fixes value checks (was COUNT(*)==1 only). A future migration that rebuilt not_found preserving the PK but corrupting the timestamp would now be caught.

(B) The "migration genuinely bites" property is now MEASURED, not just argued: each of the six previously-unmeasured migrations (v2_to_v3, v3_to_v4, v4_to_v5, v6_to_v7, v7_to_v8, v8_to_v9) was stubbed to a no-op and the cascade tests confirmed to fail, then reverted (no stub committed). All six bite; no coverage gap found.

Test-only change; zero production impact. 851 workspace tests, dual gate GO/GO.

Commit: d986c75 (batched with task-0182).
<!-- SECTION:FINAL_SUMMARY:END -->
