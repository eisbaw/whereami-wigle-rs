---
id: TASK-0062
title: Replace stringly-typed wire fields with enums
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-10 10:57'
updated_date: '2026-05-10 16:33'
labels:
  - refactor
  - types
  - wire-format
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Two places where string vocabularies cross the wire and the type erasure is fragile: (1) server.rs:705 / client/lib.rs:128 db_status: String with values 'cached'|'pending'|'not_found'|'new'; (2) server.rs:642-654 ResolveResult.source: String with 'api'|'cache'|'not_found' — different vocabulary from the DB Source enum's 'apple'|'wigle'|... Two parallel string sets under the same field name 'source'. Replace both with serde-Serialize/Deserialize enums; rename the protocol field to 'provenance' to disambiguate from data Source. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 DbStatus enum (Cached, Pending, NotFound, New) used in DebugBssid wire type
- [x] #2 Provenance enum (Api, Cache, NotFound) used in ResolveResult; field renamed to 'provenance' (or kept as 'source' with a doc comment about the disambiguation)
- [x] #3 Wire format remains backward-compatible (same string values)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add DbStatus enum (Cached, Pending, NotFound, New) in server.rs with #[serde(rename_all = "snake_case")] and a Display impl for the CLI debug renderer.
2. Mirror the enum in whereami-client/src/lib.rs (DebugBssid.db_status: DbStatus).
3. Add Provenance enum (Api, Cache, NotFound) in server.rs with serde rename_all snake_case; rename ResolveResult.source -> provenance with #[serde(rename = "source")] for wire compat.
4. Mirror Provenance in client lib (ResolveResultEntry.source -> provenance with same rename).
5. Update CLI main.rs debug renderer to handle the typed enum (Display).
6. Add wire-format round-trip tests covering all variants + unknown-string fallback.
7. cargo build/test/clippy/fmt; commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Deferred during Phase 5: wire-format enums for db_status and resolve provenance. The current 4-value db_status string vocabulary ('cached'|'pending'|'not_found'|'new') and 3-value provenance ('api'|'cache'|'not_found') are stringly-typed but small and stable. Converting to enums requires wire-format compatibility shims for existing CLIs and risks breaking the 'soft' invariant that older clients silently ignore unknown values. Re-open if a typo causes a real bug.

Daemon side: DbStatus enum (Cached/Pending/NotFound/New) and Provenance enum (Api/Cache/NotFound) added with #[serde(rename_all = "snake_case")]; ResolveResult.source renamed to provenance with #[serde(rename = "source")] for wire compat. Daemon serializes strict — no #[serde(other)] — so a client mismatch surfaces loudly.

Client side: same enums mirrored, but with #[serde(other)] Unknown variants so future daemon variants don't break older clients. Display impls drive the existing CLI debug renderer.\n\nTests: 5 daemon-side wire-format tests + 6 client-side round-trip tests cover all variants, the source-field rename, and the Unknown forward-compat fallback.\n\ncargo build / test (96 tests pass) / clippy / fmt --check all green.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced two stringly-typed wire fields with serde-derived enums.

Daemon (server.rs):
- DbStatus { Cached, Pending, NotFound, New } — used by DebugBssid.db_status.
- Provenance { Api, Cache, NotFound } — used by ResolveResult; the Rust
  field is `provenance` but #[serde(rename = "source")] keeps the wire
  field name unchanged for backward compat with curl scripts and older
  CLIs. Disambiguates from the data-`Source` enum (apple/wigle/...)
  whose vocabulary is parallel.

Client (whereami-client/src/lib.rs):
- Same two enums mirrored, but with #[serde(other)] Unknown variants so
  future daemon variants don't break older clients (the soft invariant\n  flagged in the original task notes). Display impls feed the existing\n  CLI renderer.\n\nWire format unchanged: same lowercase strings, same field name. Tests\nadded on both sides covering every variant, the rename, and the\nUnknown forward-compat path. cargo build / test / clippy / fmt all clean.\n\nGotcha: daemon-side enums are intentionally strict (no #[serde(other)]).\nIf we later add a new variant we MUST roll the client first or accept\na hard error on older daemons that emit it. The asymmetry is\nintentional — strict on the originator, tolerant on the consumer.
<!-- SECTION:FINAL_SUMMARY:END -->
