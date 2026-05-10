---
id: TASK-0062
title: Replace stringly-typed wire fields with enums
status: To Do
assignee: []
created_date: '2026-05-10 10:57'
updated_date: '2026-05-10 14:15'
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
- [ ] #1 DbStatus enum (Cached, Pending, NotFound, New) used in DebugBssid wire type
- [ ] #2 Provenance enum (Api, Cache, NotFound) used in ResolveResult; field renamed to 'provenance' (or kept as 'source' with a doc comment about the disambiguation)
- [ ] #3 Wire format remains backward-compatible (same string values)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Deferred during Phase 5: wire-format enums for db_status and resolve provenance. The current 4-value db_status string vocabulary ('cached'|'pending'|'not_found'|'new') and 3-value provenance ('api'|'cache'|'not_found') are stringly-typed but small and stable. Converting to enums requires wire-format compatibility shims for existing CLIs and risks breaking the 'soft' invariant that older clients silently ignore unknown values. Re-open if a typo causes a real bug.
<!-- SECTION:NOTES:END -->
