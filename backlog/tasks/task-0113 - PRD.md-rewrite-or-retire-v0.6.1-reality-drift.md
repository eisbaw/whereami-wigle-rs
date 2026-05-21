---
id: TASK-0113
title: PRD.md rewrite or retire (v0.6.1 reality drift)
status: To Do
assignee: []
created_date: '2026-05-20 21:13'
labels:
  - docs
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD.md is stale. Reading cold reveals: 'spherical-mean centroid' (v0.6.1 changed this to two-stage filter); no mention of --http-timeout-secs, --nominatim-timeout-secs, --address-cache-ttl-days, --history-retention-days, --history-segment-distance-m, --history-segment-min-duration-secs; locate example response missing address/stale/age_s/pending/visible/stable; stats response missing inflight/db_write_failures; resolve response says source: 'not_found' without flagging Source/Provenance terminology split (task-0062). Task-0068 covered v0.4.0 era; another sweep is due.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Option A: rewrite PRD.md to match v0.6.1 reality — every CLI flag, every wire response shape, every Source vs Provenance distinction
- [ ] #2 Option B: retire PRD.md and replace with a one-page pointer to README.md, docs/protocol.md, and CHANGELOG.md (the actual sources of truth)
- [ ] #3 Decision recorded in task final-summary; pick one
- [ ] #4 If kept, add a CHANGELOG entry noting PRD refreshed and the version it now reflects
<!-- AC:END -->
