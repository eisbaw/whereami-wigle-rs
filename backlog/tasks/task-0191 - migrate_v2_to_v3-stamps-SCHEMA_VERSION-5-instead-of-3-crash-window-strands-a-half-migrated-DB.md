---
id: TASK-0191
title: >-
  migrate_v2_to_v3 stamps SCHEMA_VERSION (5) instead of 3 - crash window strands
  a half-migrated DB
status: Done
assignee:
  - '@claude'
created_date: '2026-07-02 04:50'
updated_date: '2026-07-02 23:20'
labels:
  - correctness
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Both stamp sites in the v2->v3 migration write params![SCHEMA_VERSION] (=5) rather than 3. Normally the in-memory chain heals it microseconds later, but a crash/power-loss/OOM between the v2->v3 commit and the v3->v4 commit leaves the on-disk version at 5 while last_fix (v4) and fixes (v5) do not exist. Every later open reads 5 and skips both migrations: get_last_fix warns, set_last_fix/insert_fix fail on every locate, history returns db errors - permanently, with no self-heal. Ref whereamid/src/db.rs:277-288,308-314 vs chain driver 149-165.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 after migrate_v2_to_v3 commits in isolation the stored schema_version is exactly 3 (keep the version<3 guard)
- [x] #2 a step-by-step migration test from a v2 fixture observes version stamps 2->3->4->5 with the matching tables present at each step
- [x] #3 a DB stamped newer than its actual shape is detected (error or repair) instead of silently skipping migrations
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Fix both stamp sites in migrate_v2_to_v3 to write literal 3 (not SCHEMA_VERSION):
   - rebuild path: UPDATE ... SET version = 3 WHERE id = 1 AND version < 3
   - already-constrained path: INSERT OR IGNORE (1,3) then UPDATE ... version = 3 WHERE id = 1 AND version < 3 (no-downgrade, no PK-violation on a v4/v5 row reached out of order)
   - mirror migrate_v1_to_v2 comment: a migration stamps its OWN postcondition, never the moving SCHEMA_VERSION constant.
2. AC#3 shape-repair at START of migrate(): if stored version <= SCHEMA_VERSION and its table shape is missing (version>=4 requires last_fix; version>=5 requires fixes), warn loudly and reset stored version DOWN to highest satisfied shape, then let the idempotent chain re-run and rebuild. Keep the version>SCHEMA_VERSION forward-compat warning untouched (never repair a genuinely-newer future schema down). Add table_exists() + highest_satisfied_shape_version() helpers.
3. Tests (db.rs test module, private fns reachable):
   - AC#1/#2 step-through: build genuine v2 (create_schema_v1 + migrate_v1_to_v2), apply v2->v3/v3->v4/v4->v5 one at a time; assert stamp 3->4->5 and table presence (v3: constrained schema_version, no last_fix/fixes; v4: last_fix, no fixes; v5: fixes).
   - AC#3 repair: v2 -> v2->v3 -> manual UPDATE version=5 -> migrate() heals (last_fix+fixes exist, version 5).
   - AC#3 negative: intact v5 passes untouched.
4. Gate: nix develop -c just e2e + fmt. Commit fix(db): ... (no backlog files).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented in commit 1078900 (fix(db): stamp v2->v3 migration at literal 3).
- Both stamp sites now write literal 3. Rebuild path: guarded UPDATE version=3 WHERE version<3. Already-constrained path rewritten as INSERT OR IGNORE (1,3) + guarded UPDATE, which avoids a PRIMARY KEY violation the naive "if updated==0 then INSERT" would hit on a v4/v5 row (a subtlety beyond the raw steer; documented in-code).
- Shape-repair added at top of migrate() with table_exists() + highest_satisfied_shape_version() helpers; only fires when version<=SCHEMA_VERSION and shape missing (v>=4 needs last_fix, v>=5 needs fixes). version>SCHEMA_VERSION left untouched (forward-compat).
- Gotcha for future work: repair writes via UPDATE ... WHERE id=1; safe because repair only triggers at version>=4 which implies the v3 single-row shape. A schema_version table manually corrupted to a non-id shape while stamped >=4 is out of contract and fails loudly (not silently). Documented in-code.
- Gate green: 272 tests (7+22+1+82+147+11+2), clippy clean, fmt clean.

Review gate (orchestrator, 2026-07-03): qa-test-runner GO (279 tests green at 3d6dbd6, clippy re-verified on a FRESH target — 0 warnings, fmt clean, migration+perm tests 2x + 5x stress with zero flakes, no temp-dir leaks). mped-architect GO on all 3 ACs; explicitly endorsed the INSERT OR IGNORE deviation as a genuine improvement (the steered updated==0/INSERT structure PK-violates on an out-of-order (1,>=3) row); walked all crash windows of the shape-repair — crash-safe, re-entrant, cannot downgrade healthy or future DBs. In-thread hardening 53aba50: softened the shape-repair comment (re-keyed row makes the repair UPDATE a zero-row no-op retried next open, not a loud failure).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Fixed migrate_v2_to_v3 stamping SCHEMA_VERSION (=5) instead of its own postcondition version 3, which could permanently strand a DB.

What & why:
- Root cause: both stamp sites in migrate_v2_to_v3 wrote params![SCHEMA_VERSION]. A crash between the v2->v3 commit and the v3->v4 commit left version=5 on disk with no last_fix (v4) / fixes (v5) tables; every later open read 5, skipped both migrations, and broke set_last_fix/insert_fix on every locate with no self-heal.
- Fix: stamp the literal 3 at both sites (mirroring migrate_v1_to_v2). Rebuild path uses a guarded UPDATE (version < 3) so a higher malformed value is kept, not downgraded. The already-constrained path was rewritten as INSERT OR IGNORE (1,3) + guarded UPDATE, which is idempotent and avoids the PRIMARY KEY violation a naive if-updated==0-then-INSERT would hit on a v4/v5 row reached out of order.
- Self-heal (AC#3): a shape-vs-stamp repair at the start of migrate() resets a too-new stamp DOWN to the highest satisfied shape (v>=4 needs last_fix, v>=5 needs fixes) and re-runs the idempotency-guarded chain. A version strictly > SCHEMA_VERSION (a genuinely-newer future schema) is deliberately left untouched.

User impact: half-migrated DBs now self-heal on next open instead of failing every locate forever.

Tests (all green): step-through v2->3->4->5 asserting stamp + table presence at each step; repair of a forged version=5/no-tables strand; intact-v5 no-op; future-schema (99) untouched.

Risk/limits: repair writes via UPDATE ... WHERE id=1, safe because it only fires at version>=4 (which implies the v3 single-row shape); a schema_version table manually corrupted to a non-id shape while stamped >=4 is out of contract and fails loudly rather than silently mis-heals (documented in-code).

Commit: 1078900.
<!-- SECTION:FINAL_SUMMARY:END -->
