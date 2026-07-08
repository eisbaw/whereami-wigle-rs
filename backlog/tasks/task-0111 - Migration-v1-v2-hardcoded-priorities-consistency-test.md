---
id: TASK-0111
title: Migration v1->v2 hardcoded priorities consistency test
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:12'
updated_date: '2026-07-08 19:11'
labels:
  - correctness
  - migrations
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/db.rs:225-232. The v1->v2 backfill hardcodes source priority numbers in SQL, with a comment 'do not call Source::priority() here'. This is correct discipline (migrations are frozen archaeology, not live code). But there's no test asserting the hardcoded migration numbers EQUAL Source::priority() AT THE TIME OF MERGE — they have to be equal until/unless a future change deliberately diverges. If Apple is renumbered to 50 in the enum, the migration would silently set old rows to the old number with no signal.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Unit test: run migrate_v1_to_v2 on a fresh DB with one row per Source string variant, then assert each row's source_priority equals Source::from_str(s).unwrap().priority()
- [x] #2 Doc-comment on migrate_v1_to_v2 explicitly says: 'these constants must equal Source::priority() AT THE TIME THIS MIGRATION SHIPPED; later renumbering of the enum requires a new migration, not editing this one'
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add test migrate_v1_v2_priorities_match_source_enum in db.rs tests module: build in-memory Database + create_schema_v1, insert one aps row per source string (apple/wigle/beacondb/manual/legacy_prototype) with distinct 02:00 BSSIDs, call migrate_v1_to_v2, read back source_priority and assert == Source::from_db_str(source).priority().
2. Strengthen migrate_v1_to_v2 doc-comment: constants must EQUAL Source::priority() at ship time; renumbering needs a NEW migration; task-0111 pins equality.
3. Verify: cargo test -p whereamid (lib+bin), just e2e, clippy -D warnings, fmt --check. Run new test twice.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented AC#1 (test migrate_v1_to_v2_priorities_match_source_enum) + AC#2 (strengthened doc-comment). Committed 74a24d0 (code only; backlog md left dirty).

v1 aps INSERT shape used: (bssid, lat, lon, source, first_seen, last_seen, fetched_at) VALUES (?, 0.0, 0.0, ?, 't', 't', 't') — the seven NOT NULL v1 cols; no source_priority (migration adds it).

Gates: whereamid tests 201 lib + 459 bin (new test passes in BOTH targets), 12 proptests, 4 shutdown, all 0 failed. New test run twice = ok. clippy --all-targets -D warnings exit 0. cargo fmt --check exit 0. just e2e exit 0. Left In Progress for review gate; ACs NOT checked.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a consistency tripwire pinning the v1->v2 migration's HARDCODED source priorities to the live Source::priority() enum at ship time. The migration deliberately hardcodes 40/30/20/10/0 in SQL (frozen archaeology — must not silently rewrite historical rows if the enum is renumbered), but nothing asserted the frozen numbers equalled the enum at merge time.

Changes (commit 74a24d0, whereamid/src/db.rs — TEST + one doc-comment, no behavior change):
- Test migrate_v1_to_v2_priorities_match_source_enum: create_schema_v1() in-memory, insert one aps row per source string (apple/wigle/beacondb/manual + an unrecognized "legacy_prototype"), run migrate_v1_to_v2(), assert each row's source_priority == Source::from_db_str(source).priority(). (The unrecognized string -> Unknown -> 0 pins the SQL NOT IN(...) -> 0 branch too.)
- Strengthened the migrate_v1_to_v2 backfill doc-comment: constants must EQUAL Source::priority() at ship time; a later renumbering requires a NEW migration, not editing this one.

Note: the task text referenced Source::from_str(s).unwrap() but the real API is Source::from_db_str(s) -> Source (no Result); used the real API — AC intent fully met.

Verification: test+doc-only diff (git-confirmed), orchestrator-verified: db lib 201 / bin 459 (0 failed), new test passes in BOTH dual-target binaries, clippy -D warnings + fmt clean, e2e green. BITE MUTATION-CONFIRMED by orchestrator: setting Source::Apple => 50 makes the test FAIL ("migration backfill for source apple (40) must equal Source::priority() (50); ... renumber via a NEW migration"), then restored. Known coverage boundary (not a defect): the test pins the 5 inserted string cases, not data-driven off the enum (no strum/iterator) — adding a new Source variant is itself a ladder change that should ship its own migration+test.
<!-- SECTION:FINAL_SUMMARY:END -->
