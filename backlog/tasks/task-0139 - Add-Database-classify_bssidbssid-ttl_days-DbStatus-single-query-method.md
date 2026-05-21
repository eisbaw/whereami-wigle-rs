---
id: TASK-0139
title: 'Add Database::classify_bssid(bssid, ttl_days) -> DbStatus single-query method'
status: To Do
assignee: []
created_date: '2026-05-20 21:23'
labels:
  - refactor
  - db
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:805-861. handle_debug iterates BSSIDs and calls get_ap + (maybe) is_not_found + (maybe) is_pending per BSSID — up to 3 DB lock acquires × N. Task-0091 is about releasing the debouncer mutex but the underlying anti-pattern persists. Conceptually each BSSID has one DbStatus state (Cached(ApInfo) | NotFound | Pending | New), discoverable in one query.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Database::classify_bssid(bssid, ttl_days) -> Result<DbStatus> with one SQL using LEFT JOIN aps/not_found/pending
- [ ] #2 handle_debug calls classify_bssid once per BSSID under one DB lock acquisition
- [ ] #3 Existing handle_debug tests pass; new test asserts the single-query semantics by stubbing the DB and counting queries OR checking lock count
- [ ] #4 Resolver chain may also use classify_bssid (collapses the existing get_ap + is_not_found + is_pending probes into one)
<!-- AC:END -->
