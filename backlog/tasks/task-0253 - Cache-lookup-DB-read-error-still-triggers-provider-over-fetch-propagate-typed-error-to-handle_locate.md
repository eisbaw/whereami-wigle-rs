---
id: TASK-0253
title: >-
  Cache-lookup DB read error still triggers provider over-fetch; propagate typed
  error to handle_locate
status: To Do
assignee: []
created_date: '2026-07-08 17:22'
labels:
  - robustness
  - correctness
dependencies:
  - TASK-0105
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0105 review. task-0105 made a swallowed cache-read DB error VISIBLE (db_read_failures counter) but did NOT change control flow: lookup_cached still returns empty on Err, so handle_locate treats it as a cache miss, kicks the background resolver, and burns API quota re-fetching rows that are actually still in the DB (the original task-0105 motivation). AC#2 explicitly sanctioned the observability-only path ("increment ... OR return a typed result"), so this is a distinct, still-open improvement, not a task-0105 defect. Fix direction: propagate a typed error (not an empty Vec) from lookup_cached/lookup_cached_readonly to handle_locate so a genuine DB read error is distinguishable from a real miss, and DECIDE the correct behavior on read error — note "do not kick the resolver on read error" is non-obvious (you would then neither serve a fix nor retry), which is exactly why it needs its own design task. Consider surfacing a degraded/error response to the client rather than a silent quota-burning refetch.
<!-- SECTION:DESCRIPTION:END -->
