---
id: TASK-0031
title: Add location history timeseries DB
status: To Do
assignee: []
created_date: '2026-05-09 21:57'
labels:
  - feature
  - history
  - db
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Persist daemon-resolved fixes as a time-series so users can query 'where was I 7 days ago'. Group adjacent fixes into location segments (stay points) rather than storing every raw fix. Provide CLI/HTTP commands to query history by time range and to list segments.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 fixes table records (timestamp, lat, lon, accuracy_m, source) for each successful locate
- [ ] #2 segmentation logic groups consecutive fixes within a configurable distance + duration threshold into a single segment (start, end, centroid, accuracy)
- [ ] #3 configurable retention policy (e.g. --history-retention-days) prunes old fixes
- [ ] #4 CLI command 'whereami history <range>' returns segments for the range (e.g. '7d', '24h', ISO range)
- [ ] #5 HTTP cmd: 'history' returns segments as typed JSON
- [ ] #6 schema migration is idempotent and respects the schema_version single-row invariant from task-0030
- [ ] #7 writes to fixes table do not block the locate hot path (best-effort, async)
- [ ] #8 tests cover segmentation thresholds (single fix, dispersed fixes, contiguous stay, midnight crossover)
<!-- AC:END -->
