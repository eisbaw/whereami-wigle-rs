---
id: TASK-0238
title: >-
  Reject non-finite lat/lon at the LastFix rehydration and resolve serve
  boundaries
status: Done
assignee:
  - '@myself'
created_date: '2026-07-04 17:18'
updated_date: '2026-07-04 22:30'
labels:
  - correctness
  - robustness
dependencies:
  - TASK-0227
  - TASK-0106
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Sibling to task-0227/0106 (found in their review gate). The same NaN-from-SQLite-REAL bug class exists on paths that BYPASS the new trilaterate + address_cache guards: TryFrom<LastFixRow> for LastFix (server.rs ~188-205) validates only the timestamp and copies row.lat/row.lon verbatim; main.rs (~107-120) accepts it into initial_last_fix; return_last_fix_or_error (server.rs ~1030-1049) serves fix.lat/lon; handle_resolve (server.rs ~1078-1089) returns cached ap.lat/lon unfiltered. A poisoned last_fix or aps REAL column is therefore served on restart and via resolve. Impact is bounded (serde_json coerces non-finite f64 to JSON null -> degraded, not a wrong number), so this is fail-not-fast, not served-garbage. Root-cause fix: reject non-finite in TryFrom<LastFixRow> (discard the row like a bad timestamp already is) as the single choke point; it also subsumes the fix.lat-non-finite case behind task-0106 backfill guard. Line numbers approximate.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 TryFrom<LastFixRow> for LastFix returns Err (row discarded, like a bad timestamp) when lat or lon is non-finite, with a warn! noting the discard
- [x] #2 handle_resolve does not emit a cached AP position with non-finite lat/lon (skipped or reported unresolved), consistent with task-0227's locate-path filter
- [x] #3 unit test: a LastFixRow with NaN/Inf lat or lon rehydrates as None; a resolve for a BSSID with a non-finite cached position emits no non-finite coordinate
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a fail-closed finite lat/lon check at LastFix::try_from so main.rs's existing discard warning names the conversion error and startup rehydrates no poisoned fix.
2. Guard every positioned result at the resolve wire boundary; warn and return null coordinates with source=unresolved for non-finite rows, while preserving finite cache/API provenance.
3. Add focused conversion tests for NaN and both infinities in either coordinate plus SQLite-backed full-dispatch tests proving poisoned cached APs are unresolved/null and finite cache hits remain cache coordinates.
4. Run focused tests, the full nix develop -c just e2e gate, and nix develop -c just fmt; leave uncommitted and In Progress for the required parallel reviewer gate.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Started implementation. Confirmed existing main.rs maps LastFix::try_from errors to None and warns `discarding persisted last_fix: {e}`; no second warning is needed in the conversion. Confirmed resolve must explicitly classify a poisoned positioned row as unresolved rather than falling through authoritative not_found logic.

Implemented the two choke-point guards and focused regressions. LastFix::try_from now returns an error naming non-finite lat/lon; main.rs already turns that error into None and emits `discarding persisted last_fix: {e}`. handle_resolve now maps any positioned non-finite result explicitly to null lat/lon and source=unresolved, with a warning, so it cannot fall through to not_found. Focused results: rehydration filter 1 passed; resolve-dispatch filter 5 passed (including +Inf latitude, -Inf longitude, and finite cache control). Gotcha/rejected approach: asserting JSON null alone is insufficient because serde_json masked the old defect that way; the SQLite-backed regression also asserts source=unresolved. Used infinities because SQLite can normalize NaN to NULL.

Full local gate is green on the uncommitted diff: `nix develop -c just e2e` ran 322 tests (15 whereami-client lib + 22 whereami CLI + 1 SIGPIPE + 88 whereamid lib + 181 whereamid main + 12 property + 3 shutdown), all passed with 0 failures/ignored, then strict `cargo clippy --all-targets -- -D warnings` completed with no findings. `nix develop -c just fmt` also exited 0. Leaving ACs unchecked and status In Progress until the required independent qa-test-runner + mped-architect review gate. No blocker, shortcut, or out-of-scope defect was discovered; no follow-up task or forward-carry target is warranted from this narrow fix.

Independent pre-commit review gate: qa-test-runner GO with 322/322 tests, 0 failed/ignored, strict clippy -D warnings clean, fmt clean, git diff --check clean, and all three focused regressions passing twice (6/6 invocations). mped-architect GO with no blocking findings; confirmed the single startup warning satisfies AC#1 without duplicating logging inside TryFrom, the tests fail on old behavior despite serde null-masking, and finite cache/API behavior and request ordering remain intact.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Reject non-finite coordinates at the two remaining serve boundaries discovered during the task-0227 review.

Changes:
- LastFix::try_from now rejects NaN/±Inf latitude or longitude, so startup discards a poisoned persisted row and reports the existing root-cause warning instead of exposing it as a stale fix.
- handle_resolve now warns and maps any non-finite positioned result to null coordinates with source=unresolved; poisoned cache data can no longer masquerade as cache/API success or authoritative not_found.
- Added a six-case NaN/±Inf rehydration matrix, SQLite-backed resolve regressions, and a finite cache positive control.

Tests:
- Independent qa gate: 322/322 workspace tests passed; strict clippy -D warnings, fmt, and git diff --check clean.
- Three focused regressions passed twice (6/6 invocations).
- Independent architecture review: GO, no blocking findings.

Risk/follow-up:
- SQLite-backed wire tests deliberately use infinities because bound NaN may normalize to NULL; direct row conversion covers NaN. The separate locate-boundary observability gap remains tracked by TASK-0239.
<!-- SECTION:FINAL_SUMMARY:END -->
