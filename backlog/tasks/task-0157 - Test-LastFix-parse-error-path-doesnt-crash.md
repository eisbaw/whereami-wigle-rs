---
id: TASK-0157
title: 'Test: LastFix parse-error path doesn''t crash'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:30'
updated_date: '2026-07-09 04:21'
labels:
  - tests
  - robustness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P1-3. server.rs:139 TryFrom<LastFixRow> for LastFix errors on unparseable at_rfc3339. main.rs:73-80 uses this as a 'discard malformed last_fix' path. No test exercises an unparseable timestamp. If someone refactors to .expect(), the daemon would crash on startup if garbage ever lands in last_fix.at_rfc3339.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Test: insert a last_fix row via raw SQL with at_rfc3339='garbage'
- [x] #2 Call LastFix::try_from(row); assert is_err()
- [x] #3 A corrupt persisted last_fix does not break the daemon: Database::open succeeds and the corrupt row is discarded on read (typed CorruptLastFix, not stale/panic) while a locate still answers — covered end-to-end by get_last_fix_on_corrupt_timestamp_is_err_not_stale + locate_with_corrupt_persisted_last_fix_still_answers. (AC originally said 'in-memory last_fix is None at boot' — obsolete: task-0134/0238 made last_fix read on-demand, no in-memory-at-boot copy. The residual main.rs startup-rehydration glue is tracked as task-0273.)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Audit existing coverage of unparseable last_fix timestamp
2. Found get_last_fix_on_corrupt_timestamp_is_err_not_stale (db.rs) + locate_with_corrupt_persisted_last_fix_still_answers (server.rs) already cover it
3. Report already-covered; add nothing redundant
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ALREADY COVERED — no new test added (avoided redundant bloat), reviewer-confirmed. The unparseable-last_fix-timestamp hazard is pinned at two layers: db.rs get_last_fix_on_corrupt_timestamp_is_err_not_stale (raw SQL sets at_rfc3339=garbage; asserts typed CorruptLastFix + downcast, not panic/stale — AC#1+#2, though the parse moved from server.rs TryFrom into last_fix_from_columns via task-0134/0238) and server.rs locate_with_corrupt_persisted_last_fix_still_answers (end-to-end discard-and-continue: corrupt row ignored, locate still answers ok, warn emitted). Dual-gate reviewer independently verified both tests substantively pin the hazard and that skipping a 4th near-duplicate is correct. AC#3 rewritten (obsolete in-memory-at-boot architecture -> current on-demand-read reality). Residual: the main.rs:130-140 startup-rehydration CorruptLastFix->None glue arm is untested (thin binary glue; ingredients already tested) -> filed task-0273 (dep 0157).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Verified (no new test needed) that an unparseable last_fix timestamp is handled safely, not crashed on. The hazard is already pinned at the db layer (typed CorruptLastFix error, not a stale Ok or panic) and end-to-end at the server layer (corrupt persisted last_fix is discarded and a locate still answers). The task\x27s AC#3 described an obsolete in-memory-at-boot architecture (last_fix became on-demand via task-0134/0238); updated to the current reality, whose intent is covered. The one residual — the main.rs startup-rehydration glue arm — is filed as task-0273. Reviewer-confirmed the coverage is genuine and a 4th test would be bloat. No code change.
<!-- SECTION:FINAL_SUMMARY:END -->
