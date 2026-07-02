---
id: TASK-0193
title: Bound and dedupe background Nominatim geocode tasks
status: To Do
assignee: []
created_date: '2026-07-02 04:51'
labels:
  - correctness
  - performance
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Every locate whose rounded position misses the address cache spawns a task with no in-flight dedup and no bound; each serialises on the rate-limit mutex held across a 1s sleep. Until the first task inserts into the cache (>=1s + HTTP, up to 30s), every further locate at that cell spawns a duplicate; if Nominatim errors/down nothing is inserted so a client polling at 2 Hz grows the queue ~1 task/s indefinitely, and a moving user (new cell each fix) does the same even when healthy. Wasted OSM traffic against a shared hardcoded UA (usage-policy risk). Ref whereamid/src/server.rs:501-563, nominatim.rs:65-74.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 N concurrent/rapid locates at one uncached position trigger exactly one Nominatim request (HashSet of in-flight grid keys, like the resolver InflightGuard)
- [ ] #2 with Nominatim unreachable the number of live geocode tasks stays bounded regardless of locate rate
- [ ] #3 requests are not issued for positions superseded more than a bounded interval ago
<!-- AC:END -->
