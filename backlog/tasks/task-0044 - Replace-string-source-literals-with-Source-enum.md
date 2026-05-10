---
id: TASK-0044
title: Replace string source literals with Source enum
status: To Do
assignee: []
created_date: '2026-05-10 05:39'
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
- [ ] #1 ApInfo.source is Source enum, not String
- [ ] #2 All call sites construct ApInfo with a Source variant; no string literals in production code
- [ ] #3 DB layer converts Source <-> string at the SQLite boundary only
- [ ] #4 Tests cover the Source <-> string round-trip including Unknown
<!-- AC:END -->
