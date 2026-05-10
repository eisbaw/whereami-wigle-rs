---
id: TASK-0065
title: 'Document Source::BeaconDb as legacy/read-only'
status: Done
assignee: []
created_date: '2026-05-10 10:57'
updated_date: '2026-05-10 14:11'
labels:
  - docs
  - types
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
After task-0034 removed BeaconDbClient, Source::BeaconDb has zero writers in production but the enum still treats it as a peer of Apple/Wigle/Manual. Document explicitly in the enum doc comment that variants past Manual are 'historical / read-only / never produced by this codebase'. Optionally consider Source::Legacy(String) for any future deprecated-but-readable source. Found in v0.4.0 review (keeper).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Source enum has a doc comment explaining BeaconDb is read-only legacy
- [x] #2 from_db_str round-trip tests still pass
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Doc comment in db/source.rs (added during task-0055 split) explicitly notes BeaconDb is read-only legacy: 'No production path writes this today (BeaconDbClient was removed in task-0034) but historical DB rows still read back at this priority.' from_db_str round-trip tests pass unchanged.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Source enum doc comment now explicitly marks BeaconDb as read-only legacy. Variant retained so historical DB rows continue reading back at priority 20.
<!-- SECTION:FINAL_SUMMARY:END -->
