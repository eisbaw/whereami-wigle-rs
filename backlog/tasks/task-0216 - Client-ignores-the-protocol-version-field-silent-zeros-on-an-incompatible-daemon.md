---
id: TASK-0216
title: >-
  Client ignores the protocol version field -> silent zeros on an incompatible
  daemon
status: To Do
assignee: []
created_date: '2026-07-02 04:54'
updated_date: '2026-07-03 05:50'
labels:
  - protocol
  - cli
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
protocol.md says the version field v is integer-bumped on incompatible changes, but the client deserializes v:u32 on every struct and never compares it. Because every payload field is #[serde(default)], a hypothetical v:2 response with renamed fields still parses 'successfully' and whereami locate prints 0.000000, 0.000000 +-0m (0 sources) rather than failing. The one failure mode versioning exists to prevent is exactly what happens. Ref whereami-client/src/lib.rs:48-78, docs/protocol.md:234-237. Complements task-0145 (daemon-side constant).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 client rejects responses with v above its supported version with an actionable message and non-zero exit
- [ ] #2 protocol.md tells client authors what to do on unknown v
- [ ] #3 a test feeds a v:2 response and asserts the error path, not zeros
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from task-0183 (Done): under --json the client now intentionally parses ONLY the `ok` field of the response (as serde_json::Value) to derive the exit code, and passes the raw line through byte-identically — it does NOT read `v`. So protocol-version checking is a concern for the TYPED/HUMAN path only (the typed structs still deserialize `v`). Any version-mismatch guard you add should live in the typed accessors / render path, not in the --json passthrough (which must remain verbatim and must not start rejecting lines based on `v`).

Forward-carried from task-0194: protocol.md now documents that new enum values in existing string fields (e.g. resolve `source`) are additive and keep v=1, and that clients should treat an unrecognised `source` value like 'unresolved' (retry) rather than erroring. The client already tolerates this for Provenance via #[serde(other)] -> Unknown. When task-0216 adds `v` handling: a daemon emitting the newly-added 'unresolved' source value against an older client must NOT trip a version-mismatch/incompatible path — it is an additive change, not a v bump.

forward-carried from task-0196: the client transport now probes MULTIPLE endpoints (WHEREAMI_SOCKET -> WHEREAMI_ADDR -> default socket paths; whereami-client/src/lib.rs resolve_target). When surfacing a protocol-version mismatch, the message should also name WHICH endpoint answered (the resolved ConnectTarget via WhereAmIClient::target()), so a user with several daemons/overrides knows which one is incompatible.
<!-- SECTION:NOTES:END -->
