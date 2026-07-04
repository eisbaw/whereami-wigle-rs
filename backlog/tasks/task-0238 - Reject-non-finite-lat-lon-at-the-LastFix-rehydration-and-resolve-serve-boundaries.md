---
id: TASK-0238
title: >-
  Reject non-finite lat/lon at the LastFix rehydration and resolve serve
  boundaries
status: To Do
assignee: []
created_date: '2026-07-04 17:18'
labels:
  - correctness
  - robustness
dependencies:
  - TASK-0227
  - TASK-0106
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Sibling to task-0227/0106 (found in their review gate). The same NaN-from-SQLite-REAL bug class exists on paths that BYPASS the new trilaterate + address_cache guards: TryFrom<LastFixRow> for LastFix (server.rs ~188-205) validates only the timestamp and copies row.lat/row.lon verbatim; main.rs (~107-120) accepts it into initial_last_fix; return_last_fix_or_error (server.rs ~1030-1049) serves fix.lat/lon; handle_resolve (server.rs ~1078-1089) returns cached ap.lat/lon unfiltered. A poisoned last_fix or aps REAL column is therefore served on restart and via resolve. Impact is bounded (serde_json coerces non-finite f64 to JSON null -> degraded, not a wrong number), so this is fail-not-fast, not served-garbage. Root-cause fix: reject non-finite in TryFrom<LastFixRow> (discard the row like a bad timestamp already is) as the single choke point; it also subsumes the fix.lat-non-finite case behind task-0106 backfill guard. Line numbers approximate.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 TryFrom<LastFixRow> for LastFix returns Err (row discarded, like a bad timestamp) when lat or lon is non-finite, with a warn! noting the discard
- [ ] #2 handle_resolve does not emit a cached AP position with non-finite lat/lon (skipped or reported unresolved), consistent with task-0227's locate-path filter
- [ ] #3 unit test: a LastFixRow with NaN/Inf lat or lon rehydrates as None; a resolve for a BSSID with a non-finite cached position emits no non-finite coordinate
<!-- AC:END -->
