---
id: TASK-0134
title: Push DateTime/usize marshalling into db.rs layer
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:21'
updated_date: '2026-07-08 22:55'
labels:
  - refactor
  - db
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:114-156 + src/db.rs:88-100. LastFix <-> LastFixRow is hand-written adapter between layers — same data, only differing chrono::DateTime<Utc> vs String and usize vs i64. The same shape mismatch is latent in FixRow.at_rfc3339: String which history::segment_fixes parses on every iteration. The parse error there is silently swallowed (Ok,Ok => …, _ => 0). DB layer is exposing rusqlite's preferred types instead of domain types.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Database methods return DOMAIN types: get_last_fix -> Option<LastFix> (with at: DateTime<Utc>); get_fixes_in_range -> Vec<Fix> (with at: DateTime<Utc>)
- [x] #2 Parse errors at the DB boundary are TYPED errors, not silent default values
- [x] #3 LastFixRow and FixRow types are eliminated (or kept internal to db.rs, never escape)
- [x] #4 history::segment_fixes consumes Vec<Fix> directly; no String parsing inside; the (Ok,Ok)|_ silent-zero handler is removed
- [x] #5 Existing tests pass; new test: corrupt at_rfc3339 in DB causes get_last_fix to return Err (not Ok(stale))
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Move domain LastFix into db.rs (lib target excludes server.rs, so db.rs cannot reference crate::server::LastFix). Add domain Fix{at:DateTime<Utc>} in db.rs. Add CorruptLastFix marker error (downcastable) to preserve the locate-path ignore-vs-abort split.
2. Eliminate LastFixRow+FixRow. Marshal RFC3339<->DateTime and the task-0238 non-finite guard INSIDE db.rs via last_fix_from_columns(); parse fixes timestamp inside the query closure (typed rusqlite error). Keep UPSERT SQL + to_rfc3339() format byte-identical.
3. DB methods return domain: get_last_fix->Result<Option<LastFix>>, get_fixes_in_range[_guarded]->Result<Vec<Fix>>, CappedFixRows.fixes:Vec<Fix>; set_last_fix/persist_fix_and_history/persist_autolocate take LastFix.
4. history.rs segment_fixes* consume &[Fix]; remove parse_from_rfc3339 + (Ok,Ok)|_=>0 silent-zero (use end.at-start.at).
5. Migrate ALL callers+tests: server.rs (handle_locate, RawHistoryFix, HistoryWork, rehydration tests->relocate non-finite guard tests into db.rs), main.rs startup (preserve both warn msgs via CorruptLastFix downcast), geocode.rs.
6. AC#5 new db.rs test: bad at_rfc3339 string round-trips SQLite -> get_last_fix Err (downcasts CorruptLastFix).
7. Gate: nix develop -c just e2e + cargo fmt --check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented in commit 35b8f3a (code only; backlog not committed). Left In Progress for the dual review gate.

Design gotchas:
- LastFix MOVED from server.rs INTO db.rs. The task hint (db.rs references crate::server::LastFix) is impossible: db.rs also compiles into the LIB target, which does not include server.rs. So the domain type must live in the lib target. server.rs now `use crate::db::LastFix`.
- CorruptLastFix marker error (downcastable via anyhow) preserves the pre-refactor ignore-vs-abort split: a poisoned persisted row (non-finite coord/accuracy OR unparseable timestamp) is a CorruptLastFix; a genuine SQLite read failure is not. main.rs keeps BOTH startup warns (discarding vs failed-to-read); handle_locate still ignores a corrupt row and answers, but aborts on a real read failure. Without this, collapsing parse into get_last_fix would have turned the locate corrupt-row path from graceful-ignore into hard-abort (silent regression).
- Row types eliminated (not just hidden): last_fix_from_columns is the single Row->domain choke point (relocated task-0238 guard). fixes timestamp parsed in the query closure -> typed rusqlite error with row id.
- Monotonic-UPSERT: SQL unchanged; at.to_rfc3339() marshalled inside upsert_last_fix + persist_fix_and_history, byte-identical +00:00 format. On-disk columns remain RFC3339 TEXT.
- Non-finite guard tests relocated server.rs -> db.rs: NaN/Inf cannot round-trip a SQLite REAL (stored NULL), so they test last_fix_from_columns directly. AC#5 corrupt-timestamp test uses a STRING (which round-trips) via execute_batch_for_test UPDATE, driving the real get_last_fix.

Migrated callers: db.rs (methods + ~20 tests), server.rs (backfill closure sig, 3 persist_*_locked helpers, handle_locate autolocate, RawHistoryFix From, HistoryWork.fixes, previous_row/history_rows helpers, ~15 tests), main.rs startup, server/geocode.rs concurrency test.

Gate: cargo test 786 passed/0 failed; clippy --all-targets -D warnings clean; cargo fmt --check clean.

Review-gate hardening (amended into 23a7b0b, same logical commit; code only, backlog not committed, not pushed):
1. Added server-level regression test dispatch_tests::locate_with_corrupt_persisted_last_fix_still_answers. Seeds cached APs + a debounced stable scan, persists a VALID last_fix, then corrupts its at_rfc3339 in place (UPDATE ... = 'not-a-timestamp'; a string round-trips the TEXT column). Drives interactive dispatch_command(&Request::Locate) with a scoped CaptureSink and asserts BOTH: (a) ok:true with a fresh lat/lon (corrupt row ignored, not fatal) AND (b) the "ignoring invalid persisted last_fix during locate" WARN fired. Pins the CorruptLastFix downcast/arm-ordering against a silent hard-abort regression. Passed twice in isolation (warn-capture stable) + in full e2e.
2. Added guard comments: (a) at last_fix_from_columns in db.rs (INVARIANT: CorruptLastFix must stay the TOP-LEVEL anyhow error; no .context() wrap in db.rs/get_last_fix/read plumbing or the downcast breaks); (b) at the handle_locate downcast arm in server.rs pointing to that invariant + the new test.

Re-verify gate: just e2e exit 0 -> cargo test 787 passed / 0 failed (was 786; +1 new test); clippy --all-targets -D warnings clean; cargo fmt --check clean. Left In Progress for coordinator to close.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Pushed DateTime/usize marshalling into the db.rs layer: DB methods now return DOMAIN types (LastFix/Fix with at: DateTime<Utc>) instead of rusqlite Row types; parse errors at the boundary are TYPED, not silently defaulted (commit 23a7b0b).

Changes (db.rs +433, server.rs/history.rs/main.rs/geocode.rs): LastFixRow/FixRow ELIMINATED (single domain representation); LastFix moved into db.rs (the lib target excludes server.rs, so the domain type must live lib-side) + new Fix domain type. get_last_fix -> Result<Option<LastFix>>, get_fixes_in_range[_guarded] -> Vec<Fix>, set_last_fix/persist_* take LastFix. New CorruptLastFix typed error + single last_fix_from_columns / parse_fix_timestamp choke points. history::segment_fixes consumes &[Fix] directly — the per-iteration parse_from_rfc3339 + the `(Ok,Ok)|_ => 0` SILENT-ZERO handler REMOVED (the real fail-not-fast bug, AC#4). All callers + ~15 tests migrated.

AC status: #1 domain types DONE; #2 typed parse errors DONE (CorruptLastFix); #3 Row types eliminated/internal DONE; #4 segment_fixes consumes Fix, silent-zero gone DONE; #5 corrupt-timestamp test DONE.

INVARIANTS PRESERVED (mped traced all end-to-end): task-0238 non-finite last_fix guard relocated verbatim into last_fix_from_columns (non-finite -> discarded, never served; startup -> None+warn); monotonic-UPSERT SQL + `WHERE excluded.at_rfc3339 >= last_fix.at_rfc3339` unchanged, at.to_rfc3339() marshalled inside db.rs is BYTE-IDENTICAL to the pre-refactor format; RFC3339 on-disk columns + range-query string params unchanged (only the Rust return type changed); DateTime<Utc> via parse_from_rfc3339().with_timezone(&Utc) preserves the instant.

CRUX (corrupt persisted last_fix must be IGNORED by locate, not abort): preserved via a downcastable CorruptLastFix distinguished from a genuine DB read error, verified to survive the DbHandle actor boundary (no .context() wrapping). Review found this branch had NO direct test -> ADDED locate_with_corrupt_persisted_last_fix_still_answers (asserts locate still returns a position AND the ignore-warn fires; stable x2) + INVARIANT guard comments at both the db.rs return site and the server.rs downcast arm forbidding .context()-wrapping.

Verification: dual review gate qa GO (787 tests, all invariant-guarding suites pass — 0238 non-finite, monotonic UPSERT, concurrency, migrations, segmentation; AC#5 corrupt-timestamp test bites) + mped GO (every invariant traced+preserved; crux mechanically sound). In-thread review hardening: the crux regression test + guard comments (commit amended). clippy -D warnings + fmt clean.
<!-- SECTION:FINAL_SUMMARY:END -->
