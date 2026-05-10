---
id: TASK-0044
title: Replace string source literals with Source enum
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 05:39'
updated_date: '2026-05-10 07:33'
labels:
  - refactor
  - types
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0026 introduced a Source enum with priority but server.rs ~line 444 still constructs an ApInfo with source = String::from('not_found') for response synthesis. That string maps to Source::Unknown (priority 0) silently. Also wigle.rs / apple.rs use Source::Wigle.as_str().to_string() — would be cleaner to keep ApInfo.source as Source enum end-to-end and only stringify at the SQLite boundary. Reduces stringly-typed surface and prevents the literal-source footgun.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 ApInfo.source is Source enum, not String
- [x] #2 All call sites construct ApInfo with a Source variant; no string literals in production code
- [x] #3 DB layer converts Source <-> string at the SQLite boundary only
- [x] #4 Tests cover the Source <-> string round-trip including Unknown
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Change ApInfo.source from String to Source enum
2. Update all producers (apple.rs, wigle.rs) to use Source::Apple / Source::Wigle directly
3. Update DB layer: serialize via Source::as_str() at write boundary, deserialize via Source::from_db_str at read boundary
4. Update server.rs:444 'not_found' literal to use Source::Unknown (was actually a synthesized response struct, not stored — figure out cleanest representation)
5. Add tests covering the Source <-> string round-trip including Unknown
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Changed ApInfo.source from String to Source enum. Producers (apple.rs, wigle.rs) now construct with Source::Apple / Source::Wigle directly. SQLite boundary: write via Source::as_str() in upsert_ap; read via Source::from_db_str() in get_ap. Existing source_priority_ladder test already covered round-trip including Unknown.

Note: server.rs ResolveResult.source is a separate concept (provenance: 'api' / 'cache' / 'not_found') belonging to the wire protocol — NOT the AP data source. Left unchanged. The earlier QA observation conflated the two; on inspection they are distinct vocabularies.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
ApInfo.source is now a typed Source enum end-to-end in production code; the SQLite boundary is the only place strings appear. Producers no longer call '.as_str().to_string()'. Existing tests updated to use Source variants. Build/test/clippy/fmt all clean. ResolveResult.source (a provenance enum: api/cache/not_found in the wire protocol) is a separate concept and intentionally untouched.
<!-- SECTION:FINAL_SUMMARY:END -->
