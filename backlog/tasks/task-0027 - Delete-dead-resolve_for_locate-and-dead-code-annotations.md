---
id: TASK-0027
title: Delete dead resolve_for_locate and dead code annotations
status: To Do
assignee: []
created_date: '2026-05-09 21:07'
labels: []
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
resolve_for_locate is dead (#[allow(dead_code)]). ResolveResult fields cached_count, fetched_count, pending_count are dead. BeaconDB is constructed but never called. Delete resolve_for_locate, clean up dead_code annotations, remove or properly annotate BeaconDB.
<!-- SECTION:DESCRIPTION:END -->
