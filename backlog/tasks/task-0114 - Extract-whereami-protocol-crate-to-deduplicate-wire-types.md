---
id: TASK-0114
title: Extract whereami-protocol crate to deduplicate wire types
status: To Do
assignee: []
created_date: '2026-05-20 21:13'
updated_date: '2026-07-02 17:28'
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
- [ ] #1 New crate whereami-protocol with Serialize+Deserialize on every wire struct + enum
- [ ] #2 Both whereamid and whereami-client depend on it (as a workspace member or path dep)
- [ ] #3 The deliberate asymmetric forward-compat (client tolerant Unknown, server strict) is preserved via either two enum types or a serde feature flag — explicitly documented in the crate's lib.rs doc-comment
- [ ] #4 Wire-format tests live in whereami-protocol/tests/; daemon + client get to delete their duplicated test suites
- [ ] #5 docs/protocol.md updates references
- [ ] #6 Existing 179 tests still pass; the daemon+client serde round-trips are now structurally guaranteed identical
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from task-0183 (Done): whereami-client now has a strict single source of truth for each request's wire bytes — every command has one *_raw() method that serializes its request struct (LocateRequest/SimpleRequest/ResolveRequest/HistoryRequest) exactly once and calls raw_command(); the typed method delegates via parse_response_line(&self.*_raw()?). If you extract a whereami-protocol crate, keep this shape: the request structs are the single definition shared by both the typed and --json-passthrough paths, so they cannot drift. Don't reintroduce per-path request construction.
<!-- SECTION:NOTES:END -->
