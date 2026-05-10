---
id: TASK-0037
title: 'Address cache: TTL and bounded size'
status: To Do
assignee: []
created_date: '2026-05-10 05:38'
labels:
  - server
  - cache
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0025 introduced a (lat,lon)-rounded address cache to avoid blocking locate on Nominatim. The cache currently has no TTL and no size bound; daemon restart is the only invalidation. For long-running daemons street/POI data goes stale and the HashMap grows unboundedly. Add a TTL (e.g. 7 days) and a size cap (e.g. 1024 entries) with simple LRU eviction or oldest-first.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Cache entries expire after a configurable TTL (default 7 days)
- [ ] #2 Cache size is bounded; oldest entries evicted when bound is reached
- [ ] #3 Existing 'address attached on second locate' behavior preserved within TTL
- [ ] #4 Unit test covers TTL expiry and size eviction
<!-- AC:END -->
