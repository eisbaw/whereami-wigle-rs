---
id: TASK-0117
title: Drop dead/always-zero fields from LocateResponse wire shape
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:14'
updated_date: '2026-07-08 21:18'
labels:
  - refactor
  - wire-format
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:197-218. LocateResponse has nine integer fields (sources, cached, fetched, pending, visible, stable, …) — most of which are zero on most calls. fetched is always 0 (locate is cache-only; fetching happens in background). pending is always 0. These are dead wire fields leaking the old protocol shape that no longer reflects daemon behavior. Clients reading resp.fetched and expecting it to be meaningful are getting useless data.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Audit each integer field for is-it-ever-non-zero in the current code
- [x] #2 Drop fields proved-dead (likely fetched, pending, possibly others)
- [ ] #3 Optional: move remaining counts into an optional counts sub-object so future fields don't grow the response struct
- [x] #4 Update docs/protocol.md to match
- [x] #5 Wire-format test: existing JSON shape minus the dropped fields decodes cleanly on the client side (test_state and proptests pass)
- [x] #6 CHANGELOG note about wire field removal (backward-compatible since clients with serde(default) treat absent as 0)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. AC#1 audit CONFIRMED: fetched/pending literal 0 at both server ctors (2173/2174, 2240/2241); sources/cached/visible/stable live (source_count/cached_count/visible/stable_count). Client has NO deny_unknown_fields (per-field serde(default)) -> safe removal.
2. server.rs: drop fetched/pending from struct (573/574) + both ctors.
3. client lib.rs: drop pub fetched/pending (269/271); update cross-test LOCATE_FIXTURE + assertions (1998/1999).
4. main.rs: drop fetched/pending from 3 test LocateResponse constructors (render_locate did NOT print them).
5. docs/protocol.md line 127 example.
6. CHANGELOG entry (backward-compatible rationale).
7. Gate: nix develop -c just e2e, cargo fmt --check, per-crate tests. Commit code+docs, no push, no AI attribution.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented (In Progress, left for review gate).
- AC#1 audit CONFIRMED: fetched/pending literal 0 at both server ctors (server.rs:2173/2174 stale, 2240/2241 fresh->actually fresh at 2173, stale at 2240); sources/cached/visible/stable proven live (source_count/cached_count/visible/stable_count).
- deny_unknown_fields FINDING: client LocateResponse has NONE (per-field serde(default)) -> removal safe.
- Dropped fetched/pending from: server.rs struct+2 ctors; client lib.rs struct; task-0152 daemon_wire_cross_tests LOCATE_FIXTURE + 2 assertions; 3 render_locate test constructors in main.rs (render_locate never printed them).
- docs/protocol.md locate example + CHANGELOG entry done. AC#3 (counts sub-object) SKIPPED per scope.
- Gate: just e2e PASS (whereamid lib 464, integration 204, proptests 12, shutdown 4; client 49+39+2+1+2). cargo fmt --check clean. Commit fc9a5c3 (code+docs, no push).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Dropped two provably-dead always-zero integer fields (fetched, pending) from the locate wire response (commit fc9a5c3).

AUDIT (AC#1): both were HARDCODED literal 0 at the only two LocateResponse construction sites (server.rs ~2163 live-fix, ~2228 stale-fallback) — locate is cache-only, so fetching/queuing happen in the background and were never reflected here. The 4 other counts are genuinely live and KEPT: sources/cached = positioned_aps.len(), visible = latest scan len, stable = stable set len.

Changes (AC#2): removed fetched/pending from whereamid LocateResponse struct + both ctors; whereami-client LocateResponse struct; the task-0152 daemon_wire_cross_tests LOCATE fixture + its two assertions; the client render_locate test constructors (render_locate never PRINTED them, so no human-output change); docs/protocol.md locate example (AC#4); CHANGELOG (AC#6). AC#3 (counts sub-object) skipped per scope.

BACKWARD-COMPAT (verified both directions): client LocateResponse has NO deny_unknown_fields and every field is #[serde(default)]. NEW client <- OLD daemon still emitting the fields: unknown fields ignored -> clean. OLD client <- NEW daemon omitting them: serde default 0 -> clean. CHANGELOG claim accurate.

Verification: dual review gate. qa GO (777 tests 0 failed; clippy -D warnings + fmt clean; 0152 locate cross-test passes with updated fixture asserting the 4 live counts; grep sweep confirms no locate-path fetched/pending emission/doc remains; pending_aps/pending_deleted/DbStatus::Pending correctly untouched). mped GO (audit confirmed: only 2 ctor sites, all-0; kept fields live; backward-compat both directions; --json absence-assertion now more-correct not stale; no collateral; protocol.md accurate). Filed task-0261 (LOW: sources==cached are both positioned_aps.len() on the primary path — a latent redundancy of the same class, pre-existing, audit-before-collapse).

Minor accepted: a non-serde jq-style consumer that read .fetched/.pending now gets null instead of the meaningless hardcoded 0 — does not warrant a WIRE_VERSION bump.
<!-- SECTION:FINAL_SUMMARY:END -->
