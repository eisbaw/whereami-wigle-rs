---
id: TASK-0134
title: Push DateTime/usize marshalling into db.rs layer
status: To Do
assignee: []
created_date: '2026-05-20 21:21'
labels:
  - refactor
  - db
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:114-156 + src/db.rs:88-100. LastFix <-> LastFixRow is hand-written adapter between layers — same data, only differing chrono::DateTime<Utc> vs String and usize vs i64. The same shape mismatch is latent in FixRow.at_rfc3339: String which history::segment_fixes parses on every iteration. The parse error there is silently swallowed (Ok,Ok => …, _ => 0). DB layer is exposing rusqlite's preferred types instead of domain types.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Database methods return DOMAIN types: get_last_fix -> Option<LastFix> (with at: DateTime<Utc>); get_fixes_in_range -> Vec<Fix> (with at: DateTime<Utc>)
- [ ] #2 Parse errors at the DB boundary are TYPED errors, not silent default values
- [ ] #3 LastFixRow and FixRow types are eliminated (or kept internal to db.rs, never escape)
- [ ] #4 history::segment_fixes consumes Vec<Fix> directly; no String parsing inside; the (Ok,Ok)|_ silent-zero handler is removed
- [ ] #5 Existing tests pass; new test: corrupt at_rfc3339 in DB causes get_last_fix to return Err (not Ok(stale))
<!-- AC:END -->
