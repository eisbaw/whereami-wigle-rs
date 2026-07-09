---
id: TASK-0132
title: 'FixRow dead_code on id/n_sources: widen consumer or narrow row'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:21'
updated_date: '2026-07-09 00:52'
labels:
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/db.rs:76-86. FixRow.id and FixRow.n_sources carry #[allow(dead_code)] since segment_fixes doesn't use them. Same anti-pattern task-0060 (PendingAp removal) just fixed: struct wider than any consumer needs. Either expose n_sources in the history response (so users can filter low-confidence segments) or narrow the row.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Decision in task notes: expose n_sources in HistorySegment (preferred — more useful info) OR narrow FixRow
- [x] #2 If exposed: HistorySegment gains mean_n_sources field on the wire; segment_fixes averages it
- [ ] #3 If narrowed: FixRow shrinks to (at, lat, lon, accuracy_m); db.rs queries select only those columns
- [x] #4 #[allow(dead_code)] removed
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. AC#1 decision: EXPOSE (add mean_n_sources to HistorySegment wire) — chosen over narrowing FixRow because n_sources is already consumed (server.rs From<Fix> for HistoryFix) and users want to filter low-confidence stay-points.
2. protocol lib.rs: append mean_n_sources: f64 to HistorySegment with #[serde(default)] (additive, back/forward compat, v stays 1).
3. history.rs flush_segment: sum n_sources over cur, set mean_n_sources = sum/len.
4. AC#4: remove struct-level #[allow(dead_code)] on Fix; DROP Fix.id (grep shows NO field read of .id anywhere; only local row-id var used for error msgs). n_sources stays (read at server.rs:2346 + flush_segment).
5. client main.rs: render mean_n_sources on segment line.
6. docs/protocol.md: document new field.
7. wire.rs: update both HistorySegment tests (dyadic 4.0 byte-pin) + assert back-compat decode (HISTORY_FIXTURE without field -> 0.0).
8. Gate: nix develop -c just e2e + cargo fmt --check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Triaged 2026-07-08: two-path decision. PREFERRED = expose (AC#2): add mean_n_sources to the HistorySegment WIRE struct (daemon server + whereami-client, serde default for back-compat) + segment_fixes averages n_sources — a wire-format ADDITION touching both crates + client rendering + tests (medium blast radius, wants a non-fatigued context + full dual review gate). Alternative = narrow FixRow (AC#3, simpler, drops the info). Recommend expose. Not started.

Implemented (commit 38fda78) — EXPOSE path (AC#1). Left In Progress for dual review gate.

GOTCHA — the dead_code trigger was NOT id/n_sources as the doc-comment claimed:
- whereamid lib.rs re-exports `pub mod db` but NOT server/history (those are binary-only via main.rs). The lib target dead-code pass only flags pub(crate) items with no in-lib reader. All the pub fields (at/lat/lon/accuracy_m/n_sources, and formerly id) are public API and are NEVER flagged.
- So the blanket #[allow(dead_code)] was really only silencing `quality` (the sole pub(crate) field). Removing the blanket surfaced exactly one error: "field quality is never read".
- Fix.id: grep found ZERO field reads of .id anywhere (only the local row-id var in the query closures, used for corrupt-row error msgs). Dropped it (task-preferred). SELECT still selects id col 0 for those error msgs.
- Fix.n_sources: already consumed at server.rs:2346 (From<db::Fix> for HistoryFix) and now also in history::flush_segment. Not dead.
- quality: kept a NARROW field-level #[allow(dead_code)] with justification (its only production reader is the binary-crate From impl, invisible to the lib-target pass). No struct-level blanket remains.

Wire (additive, confirmed other fields unchanged):
{...,"mean_accuracy_m":35.0,"n_fixes":42,"mean_n_sources":4.0} — dyadic 4.0 byte-pin holds. Back-compat: HISTORY_FIXTURE (no field) decodes to 0.0 (asserted).

Client render: `  {start} -> {end}  {lat},{lon}  ±{acc}m  {n} fixes  (~{:.1} src)  ({dur}m)`.

Gates: nix develop -c just e2e = 804 passed / 0 failed, clippy -D warnings clean; cargo fmt --check clean.

## Review gate (orchestrator, independent) — GO/GO + hardening
- QA (re-run, 2x): 804/804 base commit, zero flake; clippy 0 -D warnings; fmt clean. mean_n_sources assert + wire byte-pin + round-trip + back-compat-default(0.0) all pass.
- Architecture: all 5 items CONFIRMED — genuine f64/f64 division (history.rs:296/303/331); additive wire (appended after n_fixes, serde default, protocol.md updated); Fix.id drop SAFE (grep = 0 field reads; SELECT col order unchanged, positional get() indices intact); dead_code narrowing accurate (quality real-but-lib-invisible: server/history are bin-only, lib re-exports only db); no NaN (cur.len()>=1 guaranteed).
- MEDIUM finding (test-coverage gap I flagged pre-review AND the reviewer confirmed): the only mean test was uniform-3 (mean 3.0) + byte-pin 4.0 — both integer, so integer-division truncation would slip. HARDENED in-thread (commit 97105ab): parameterized the fix() helper -> fix_ns(); added segment_mean_n_sources_is_fractional_not_integer_truncated asserting [3,4]->3.5. History tests 37->38; full gate re-run green (805 total).
- LOW (documented, no code change): mean_n_sources=0.0 doubles as "old daemon omitted" AND "genuine zero" — a future FILTERING consumer must treat 0.0 as unknown, not low-confidence. Recorded here as an accepted, documented limitation.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Exposed mean_n_sources on the segmented history wire so callers can filter low-confidence stay-points; removed a stale dead_code allow.

Decision (AC#1): EXPOSE, not narrow FixRow (AC#3 alternative not taken) — n_sources was already carried and averaging it adds a useful confidence signal.

What changed:
- whereami-protocol HistorySegment gains mean_n_sources: f64, appended after n_fixes, #[serde(default)] (additive wire; older daemon omitting it decodes to 0.0; protocol stays v=1).
- history::flush_segment averages n_sources over the segment (genuine f64/f64 divide, mirroring mean_accuracy_m).
- db::Fix: dropped the genuinely-unused id field (grep: 0 field reads; SELECT still selects col 0 for corrupt-row error messages) and replaced the struct-level #[allow(dead_code)] with a narrow, justified one on `quality` (its only reader, the bin-crate From<db::Fix> for HistoryFix, is invisible to the lib-target dead-code pass).
- Client renders "(~{:.1} src)" per segment. docs/protocol.md documents the field.

Why: turns a carried-but-unused column into a user-facing confidence signal and makes the dead_code annotation honest/narrow.

User impact: additive — every segmented history segment now carries mean_n_sources; all other bytes unchanged; back-compat both directions via serde default.

Tests: 805 workspace (added a fractional-mean guard [3,4]->3.5 proving float division), two independent gate runs zero flake, clippy 0, fmt clean.

Known limitation: 0.0 is ambiguous (old-daemon-omitted vs genuine zero) for any future threshold-filtering consumer.

Commits: 38fda78 (feature) + 97105ab (fractional-mean test hardening).
<!-- SECTION:FINAL_SUMMARY:END -->
