---
id: TASK-0114
title: Extract whereami-protocol crate to deduplicate wire types
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:13'
updated_date: '2026-07-09 00:10'
labels:
  - refactor
  - wire-format
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Every JSON struct exists twice: whereamid/src/server.rs (175-308) AND whereami-client/src/lib.rs (24-280). LocateResponse, Provenance, DbStatus, ResolveResult/Entry, DebugBssid, ScanResponse, StatsResponse, VersionResponse, HistoryResponse, HistorySegment — all duplicated. Subtle drift today (server uses skip_serializing_if, client uses serde(default); client has serde(other) Unknown, server doesn't). Cross-side tests are missing (Scout P0-5). Extracting to a shared crate makes the wire contract a single source of truth.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 New crate whereami-protocol with Serialize+Deserialize on every wire struct + enum
- [x] #2 Both whereamid and whereami-client depend on it (as a workspace member or path dep)
- [x] #3 The deliberate asymmetric forward-compat (client tolerant Unknown, server strict) is preserved via either two enum types or a serde feature flag — explicitly documented in the crate's lib.rs doc-comment
- [x] #4 Wire-format tests live in whereami-protocol/tests/; daemon + client get to delete their duplicated test suites
- [x] #5 docs/protocol.md updates references
- [x] #6 Existing 179 tests still pass; the daemon+client serde round-trips are now structurally guaranteed identical
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Create whereami-protocol crate (Cargo.toml workspace-inherited + src/lib.rs); add to workspace members; git add early (nix flakes need tracked files).
2. Move shared WIRE types into protocol with combined serde attrs (client field-set + daemon skip_serializing_if): responses Locate/Resolve/Scan/Stats/Debug/Version/History/HistoryFixes/Wipe/Error(+ResolveResultEntry/NetworkEntry/DebugBssid/HistorySegment/HistoryRange/HistoryFix), enums Confidence(Default High)/Provenance(+Unknown other)/DbStatus(+Unknown other), request structs Locate/Simple/Resolve/History/HistoryFixes. Enum asymmetry: ONE enum with tolerant Unknown+serde(other); daemon strict-by-construction (never emits Unknown). error field: skip_serializing_if none so daemon output byte-identical.
3. Rewire whereami-client: pub use protocol types; keep client-only (WhereAmIClient/traits/WipeConfirmation/HistoryFixesShape/*_raw). quality.rs re-exports protocol::Confidence + ConfidenceDb ext trait for as_db_str/from_db_str.
4. Rewire whereamid/server.rs: import protocol types, delete local defs, add error:None + Some(...) at construction sites; history::Segment aliases protocol::HistorySegment. Keep RawHistory* parallel (documented residual, orphan rule blocks moving From<db::Fix>).
5. Move wire tests to whereami-protocol/tests/ (round-trip for every type + enum Unknown + byte fixtures); delete duplicated client suites.
6. docs/protocol.md + CHANGELOG.
7. Gate: git add; nix develop -c just e2e; cargo fmt --check; cargo build; report exact counts.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from task-0183 (Done): whereami-client now has a strict single source of truth for each request's wire bytes — every command has one *_raw() method that serializes its request struct (LocateRequest/SimpleRequest/ResolveRequest/HistoryRequest) exactly once and calls raw_command(); the typed method delegates via parse_response_line(&self.*_raw()?). If you extract a whereami-protocol crate, keep this shape: the request structs are the single definition shared by both the typed and --json-passthrough paths, so they cannot drift. Don't reintroduce per-path request construction.

forward-carried from task-0245: two `Confidence` enums are now hand-mirrored — `whereamid/src/quality.rs` and `whereami-client/src/lib.rs` — with cross-reference doc comments naming each other + this task. Both use `#[serde(rename_all="lowercase")]` over identical variants/order. INTENDED asymmetry: the client enum derives `#[default] High` (a missing locate field from an OLD daemon reads High), but history uses `Option<Confidence>` where absence = legacy unknown (None), never High. When extracting the shared crate, preserve BOTH behaviors: locate defaults to High, history stays Option. Also fold in the daemon-side `FixQuality` bundle validation if it moves.

Implemented (commit 42788ef). Extracted whereami-protocol crate (git-tracked before nix steps). Unified all wire types; combined daemon skip_serializing_if + client serde(default); error field skip-when-none so daemon output byte-identical. Enum asymmetry resolved with ONE enum + tolerant Unknown/serde(other), daemon strict-by-construction. Confidence moved to protocol; DB text encoding kept daemon-local via ConfidenceDb ext trait. history::Segment aliases HistorySegment.

Review gate (qa-test-runner + mped-architect, parallel):
- qa: PASS. just e2e (nix-instantiate + cargo test + clippy -D warnings) + fmt --check all clean. 795 tests, 0 fail. clippy 0 warnings. fmt no diffs.
- mped-architect HIGH finding: my initial RawHistory* "orphan rule blocks unification" rationale was FALSE (coherence allows impl From<db::Fix> for foreign HistoryFix since db::Fix is local). Acted on it: retargeted the From impl at the shared HistoryFix, deleted RawHistoryFix/Range/Response, daemon now serializes protocol types directly. Fixed the elided attr (skip-when-none) so it stays byte-identical, added wire_shape test proving elided omitted. Added quality.rs test pinning Confidence DB<->wire token agreement.

GOTCHA: nix build .#default is PRE-EXISTING BROKEN (nix/package.nix:41 reads cargoToml.package.version from whereamid, which is version.workspace=true since task-0260/commit 2ea9b89 -> "cannot coerce set to string"). Unrelated to this task; NOT fixed here (out of scope). cargo build + git-tracking confirm the new crate resolves; crane whole-workspace src (task-0136) will include it once that pre-existing bug is fixed.

Request structs are Serialize-only (cmd: &static str cannot derive Deserialize) -> byte-exact tests instead of round-trip. WipeRequest + WipeConfirmation kept client-side (client ergonomics, not shared). Left In Progress for dual-review gate per instructions; ACs not checked.

## Review gate (orchestrator, independent) — GO/GO
- QA (re-run gate independently): 795/795 tests pass across TWO consecutive full-workspace runs, ZERO diff (no flake); clippy 0 warnings under -D warnings; fmt clean. Confirmed whereami-protocol/tests/wire.rs present and the old duplicated suites (client stats_wire_tests/wire_enum_tests, daemon wire_enum_tests, 0152 cross-fixtures) are deleted.
- Architecture (read-only): byte-identical wire output confirmed field-by-field against 06c82e0 for every load-bearing type; enum strict-by-construction VERIFIED (Provenance built only at server.rs:2132-2166, DbStatus only at 2280-2283, DB-error arm -> New not Unknown); coherence From<db::Fix> for HistoryFix sound + compiles; round-trip layer proves bidirectional compat.
- Findings addressed in-thread (commit f9f034e): MEDIUM (decode-only fixtures let a field reorder / dropped skip_serializing_if pass) -> added re-serialize order pin for float-free fixtures (Stats/Debug/Version) + byte-shape pins for History/Wipe with dyadic floats. LOW: NetworkEntry null-emit pin; unknown-field decode tolerance restored. Protocol tests 29 -> 34, gate re-run green.
- GOTCHA for future wire work: serde_json is built here WITHOUT float_roundtrip, so re-encoding a non-dyadic float fixture (55.6761 etc) is NOT byte-stable; byte-pin wire shapes only with dyadic floats (55.5, 12.25, 0.75, 42.5, 35.0). Decode-side float assertions must use approx tolerance.
- GOTCHA: whereami-protocol added a THIRD copy risk; enforced single-source by deleting RawHistory* and retargeting From at the shared HistoryFix (orphan rule satisfied: db::Fix is local to whereamid).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Extracted a whereami-protocol workspace crate as the single source of truth for every JSON-lines wire type, deleting the drifted daemon-side (server.rs) and client-side (client/lib.rs) duplicates and rewiring both crates to depend on it.

What changed:
- New crate whereami-protocol (publish=false, workspace-inherited version/edition, serde-only). Holds all response structs, the wire enums (Provenance/DbStatus/Confidence), and the outbound request structs.
- Struct unification composes both sides serde attrs (daemon skip_serializing_if + client serde(default)) so one type serializes-as-daemon and deserializes-as-client byte-identically.
- Enum asymmetry (AC#3) preserved with ONE enum carrying the tolerant Unknown + serde(other): the daemon is strict-by-construction (never builds Unknown, so output is unchanged) while the client tolerates a future token. Documented in lib.rs.
- Single-source enforced fully: RawHistory* deleted, From<db::Fix> retargeted at the shared HistoryFix (coherence OK, db::Fix is local).
- whereami-protocol/tests/wire.rs: round-trip + byte-shape + fixture layers; duplicated per-crate suites removed. protocol.md updated.

Why: the wire contract lived in two hand-synced copies that had already drifted; a mismatch silently breaks the protocol past happy-path fixtures.

User impact: none on the wire — verified byte-identical for every type (round-trip + re-serialize order pins + hand-derived daemon fixtures). Protocol version stays v=1.

Tests: whereami-protocol 34 (incl. 5 review-hardening pins); full workspace green across two independent runs, 0 flake; clippy 0; fmt clean. nix build .#default builds whereamid 0.6.2 (crane picks up the new crate) once the separate task-0265 nix fix landed.

Risks/follow-ups: request structs are Serialize-only (cmd: &static str cant derive Deserialize) — faithfully documented, byte-pinned not round-tripped. serde_json built without float_roundtrip: non-dyadic float fixtures are decode-only.
<!-- SECTION:FINAL_SUMMARY:END -->
