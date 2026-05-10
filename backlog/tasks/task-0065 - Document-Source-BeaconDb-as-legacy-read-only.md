---
id: TASK-0065
title: 'Document Source::BeaconDb as legacy/read-only'
status: To Do
assignee: []
created_date: '2026-05-10 10:57'
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
- [ ] #1 Source enum has a doc comment explaining BeaconDb is read-only legacy
- [ ] #2 from_db_str round-trip tests still pass
<!-- AC:END -->
