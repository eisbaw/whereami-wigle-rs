---
id: TASK-0026
title: Add source priority to upsert_ap
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-09 21:07'
updated_date: '2026-05-09 22:14'
labels: []
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
When a BSSID is resolved by both Apple and WiGLE, whoever writes last wins. Apple positions are generally more accurate. upsert_ap should prefer Apple over WiGLE: only overwrite if new source has equal or higher priority (apple > wigle > beacondb > manual).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Source enum with explicit numeric priorities (Apple=40 > Wigle=30 > BeaconDb=20 > Manual=10 > Unknown=0)
- [x] #2 Schema migration v1->v2 adds aps.source_priority and backfills from existing source string (idempotent)
- [x] #3 upsert_ap only overwrites data fields when incoming source_priority >= stored; last_seen still always advances
- [x] #4 Unit tests cover full priority cross-product (apple/wigle/beacondb/manual/unknown) and migration backfill
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
$1. Add Source enum (Apple, Wigle, BeaconDb, Manual, Unknown) with explicit priority(): Apple=40, Wigle=30, BeaconDb=20, Manual=10, Unknown=0.\n2. Source::from_str maps stored strings; Source::as_str gives canonical names.\n3. Migrate schema v1 -> v2: ALTER TABLE aps ADD COLUMN source_priority INTEGER NOT NULL DEFAULT 0; backfill via UPDATE based on existing source text. Bump SCHEMA_VERSION to 2.\n4. upsert_ap: change ON CONFLICT clause to overwrite only when excluded.source_priority >= aps.source_priority; persist source_priority alongside source. (touch_ap stays as-is; not_found unaffected.)\n5. Update apple.rs, wigle.rs callsites to set ApInfo.source via Source::Apple.as_str()/Wigle.as_str() (keep wire format); add helper in db.rs.\n6. Unit test: open in-memory DB, upsert apple then wigle then beacondb then manual then apple again; assert correct row stays based on priority.\n7. cargo build/test/clippy/fmt clean.\n8. Coordinate with task-0030: 0030 may bump to v3.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Picked option (B) — single row per BSSID with source_priority column. Reasons: (1) callers only ever read one ApInfo per BSSID so option (A) would add a per-call best-row pick; (2) keeps the index on bssid PK simple; (3) trivially extensible by changing priority numbers without touching schema.

Migration backfill UPDATEs hardcode 40/30/20/10/0 instead of calling Source::priority(). This is deliberate — historical data must not silently re-rank if the enum is renumbered later. Future re-rankings need their own migration.

Wired apple.rs and wigle.rs to use Source::*.as_str() for the source string. server.rs ResolveResult.source is unrelated (HTTP response field, not ApInfo).

Manual is intentionally lowest of the recognised sources: a user mistake should not block authoritative providers from correcting it. If sticky overrides become a real product requirement, raise priority explicitly in a follow-up.

Left schema_version table as-is for now; task 0030 is next and will tighten the single-row invariant.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Add source priority to upsert_ap so authoritative fixes win

Problem: aps had a single row keyed by BSSID and an opaque `source` text
column. Whichever provider wrote last won, so a WiGLE write 5 minutes after
Apple would silently degrade the cached fix.

Fix: rank sources explicitly. Source enum with priorities Apple=40,
Wigle=30, BeaconDb=20, Manual=10, Unknown=0. Schema bumped to v2 with a new
`source_priority` INTEGER column on aps. v1->v2 migration backfills from
the existing text column with hardcoded numbers (so future renumbering of
the enum cannot rewrite history). upsert_ap rewrites data fields only
when `excluded.source_priority >= aps.source_priority`. `last_seen` is
always advanced because observation is independent of authority.

Unknown sources collapse to priority 0; nothing recognised can be displaced
by gibberish in the source column.

Verification: cargo build / test / clippy --all-targets -D warnings / fmt
all clean. Two new tests:
- upsert_ap_respects_source_priority exercises the full cross-product
  (wigle->manual->beacondb->wigle->apple->{wigle,beacondb,manual,garbage}).
- migrate_v1_to_v2_backfills_source_priority hand-builds a v1 DB and
  verifies the four canonical sources plus an unknown one backfill
  correctly.

Follow-up: task 0030 will tighten schema_version to a single-row table.
<!-- SECTION:FINAL_SUMMARY:END -->
