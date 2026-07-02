---
id: TASK-0101
title: prune_fixes(0) deletes all fixes instead of disabling retention
status: Done
assignee:
  - '@claude'
created_date: '2026-05-20 21:10'
updated_date: '2026-07-02 05:27'
labels:
  - correctness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/db.rs:888-894 + src/config.rs:90-91. --history-retention-days default 30, validated >=0, help text says '0 disables retention'. That comment is honest about the bug — prune_fixes(0) computes cutoff=now and deletes everything older than now. So 0 means 'wipe history every prune sweep', not 'keep forever'. Surprising and contradicts help text.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 prune_fixes early-returns Ok(0) when retention_days == 0 (or main.rs history-prune skips the call)
- [x] #2 Help text for --history-retention-days updated to match: '0 disables retention (no pruning)'
- [x] #3 Unit test: with retention_days=0 and 10 old fixes in DB, prune_fixes is a no-op (fixes table unchanged)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. prune_fixes: early-return Ok(0) when retention_days == 0 (negatives rejected upstream by Args::validate).
2. Update --history-retention-days help text to "0 disables retention (no pruning)".
3. Test: retention 0 with 10 old fixes -> no-op.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented in whereamid/src/db.rs + whereamid/src/config.rs.
- prune_fixes: added early-return Ok(0) when retention_days == 0. Previously cutoff = now - 0 days = now, so DELETE WHERE at_rfc3339 < now wiped the whole history on every 24h sweep.
- config.rs help text (the clap doc comment) changed to "0 disables retention (no pruning); fixes are kept indefinitely". Verified via actual `whereamid --help` render.
- main.rs history-prune loop needs no change: it already matches Ok(0) => {} (no log).
DELIBERATE CHOICE for next engineer: guard is "== 0" (matches AC exactly), not "<= 0". Negative retention is impossible in production because Args::validate (config.rs:153) fail-fasts with a bail at startup on < 0. Caveat: if a negative ever bypassed validation and reached prune_fixes, cutoff would be in the FUTURE and DELETE would wipe everything. Relying on the upstream fail-fast rather than duplicating the check here (single source of truth for the bound).

Review-gate hardening (orchestrator): changed the guard from == 0 to <= 0 so the pub prune_fixes DELETE fails safe on a negative retention (would otherwise compute a future cutoff and wipe all history). Added a negative-retention no-op test. Commit c938b1c.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
prune_fixes(0) no longer wipes the entire location history. retention_days == 0 now means "disabled - keep forever", matching the --history-retention-days help text.

Changes:
- whereamid/src/db.rs: prune_fixes early-returns Ok(0) when retention_days == 0 (was: cutoff == now, deleting every row each sweep). Doc-comment explains the behaviour and that negatives are rejected upstream by Args::validate.
- whereamid/src/config.rs: --history-retention-days help text corrected from the old (buggy) "fixes still recorded but pruned on next sweep" to "0 disables retention (no pruning); fixes are kept indefinitely". Verified via whereamid --help.

Test: prune_fixes_zero_retention_is_noop inserts 10 fixes aged 365 days, calls prune_fixes(0), asserts 0 deleted and all 10 remain. Existing fixes_prune_drops_old_rows (retention 30) still passes.

Gate: nix develop -c just e2e green (183 -> 185 aggregate; +1 test double-counted across lib+bin binaries), cargo fmt --check clean.
<!-- SECTION:FINAL_SUMMARY:END -->
