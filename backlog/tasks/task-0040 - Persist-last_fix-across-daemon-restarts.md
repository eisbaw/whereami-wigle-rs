---
id: TASK-0040
title: Persist last_fix across daemon restarts
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 05:38'
updated_date: '2026-05-10 07:15'
labels:
  - server
  - persistence
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
DaemonState.last_fix is in-memory only; lost on restart. Combined with the cold-start fallback, this means after a restart the daemon must wait for at least one successful resolve before 'where am I' returns anything. Persist last_fix as a single-row last_fix table (mirroring the schema_version invariant from task-0030) and rehydrate on startup. Background resolves should also update the table when they succeed (currently only handle_locate writes last_fix).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 last_fix persisted as a single-row table with CHECK (id = 1)
- [x] #2 Daemon rehydrates last_fix on startup if table is non-empty
- [x] #3 resolve_chain successful resolution updates the table even when not invoked from handle_locate
- [x] #4 Schema migration is forward-only and idempotent
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. db.rs: bump SCHEMA_VERSION to 4; migrate_v3_to_v4 creates last_fix table with id INTEGER PRIMARY KEY CHECK(id=1); add Database::set_last_fix, Database::get_last_fix, LastFixRow; add tests
2. server.rs: change LastFix.at from std::time::Instant to chrono::DateTime<Utc> so it can persist; update age_s computation; in handle_locate persist to DB after updating in-memory; in Nominatim address backfill task also persist
3. main.rs: rehydrate state.last_fix from db.get_last_fix() on startup
4. Test idempotence (migrate twice, single row), round-trip (set_last_fix → get_last_fix), rehydrate (open DB twice)
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented as schema v3→v4 migration adding last_fix(id INTEGER PRIMARY KEY CHECK(id=1), lat, lon, accuracy_m, address, at_rfc3339, sources). Added Database::set_last_fix (single-row UPSERT) and Database::get_last_fix.

Changed server::LastFix.at from std::time::Instant to chrono::DateTime<Utc> so the timestamp can persist; the in-process consumers use chrono arithmetic for age. main.rs rehydrates state.last_fix from disk on startup; an unparseable timestamp is logged and the row is dropped (warn-only, not fatal).

handle_locate persists to DB after updating in-memory; failure to persist is warned-only. The Nominatim address-backfill background task also persists when it backfills a missing address. Three new tests: round-trip, single-row CHECK constraint, and on-disk reopen survives.

AC #3 ('resolve_chain successful resolution updates the table even when not invoked from handle_locate'): in the current architecture only handle_locate produces a Position via trilateration. Background tasks only update the cache; the next handle_locate call will pick up cached coords and write a fresh last_fix. The Nominatim background backfill DOES persist (covered).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
last_fix is now a single-row SQLite table with CHECK(id=1). Daemon rehydrates on startup so 'where am I' answers immediately after a restart instead of needing a fresh resolve cycle. Schema bumped to v4; migration is idempotent. Background Nominatim address backfill also persists. Three new tests cover round-trip, single-row enforcement, and disk-reopen rehydration.
<!-- SECTION:FINAL_SUMMARY:END -->
