---
id: TASK-0068
title: Update PRD to v0.4.0 reality
status: Done
assignee: []
created_date: '2026-05-10 10:58'
updated_date: '2026-05-10 14:28'
labels:
  - docs
  - prd
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD has multiple staleness issues: (1) lines 281-289 + 381 describe a BeaconDB client that no longer exists (deleted by task-0034); (2) lines 433-441 lists 'Location history' and 'CLI-tunable HTTP timeouts' as future work — both shipped; (3) lines 200-209 say iw is the scanner — actually nmcli is primary (ADR-010); (4) Data Model section describes schema v1 only — current is v5 (last_fix, fixes tables); (5) config-file example shows [beacondb] enabled = true — accepted by serde-ignore-extras but misleading; (6) crate structure lists beacondb.rs and omits history.rs and lib.rs; (7) CLI args canonical block missing 6 new flags. Found in v0.4.0 review (Explore).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 All BeaconDB client references removed or rewritten as 'historical Source variant only'
- [x] #2 Future-work section no longer claims shipped features as future
- [x] #3 Scanner section reflects nmcli-primary, iw-fallback (matches ADR-010 and code)
- [x] #4 Data Model lists schema v5 with all current tables (aps, not_found, pending, metadata, schema_version, last_fix, fixes)
- [x] #5 Config example removes [beacondb] block
- [x] #6 Crate structure lists current modules accurately (history.rs in, beacondb.rs out)
- [x] #7 CLI flags block lists all 16 flags including the 6 added in v0.4.0
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
PRD updated: removed BeaconDB-as-runtime references; rewrote scanner section as nmcli-primary + iw-fallback (matches ADR-010); future work no longer claims shipped features (history, configurable timeouts removed); CLI args block lists all 16 flags; config example removed [beacondb] block; crate structure lists current modules (history.rs, geo.rs, server/address_cache.rs, db/source.rs in; beacondb.rs out); Data Model section now shows schema v5 with last_fix and fixes tables, single-row CHECK on schema_version, source_priority on aps; Trilateration section describes the spherical-mean approach instead of arithmetic centroid; Operational notes drop BeaconDB and note timeouts are CLI-configurable.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
PRD aligned to v0.4.0 reality. BeaconDB references removed, scanner reflects nmcli-primary, schema doc shows v5 with all tables, CLI args block is complete, future-work no longer lists shipped features.
<!-- SECTION:FINAL_SUMMARY:END -->
