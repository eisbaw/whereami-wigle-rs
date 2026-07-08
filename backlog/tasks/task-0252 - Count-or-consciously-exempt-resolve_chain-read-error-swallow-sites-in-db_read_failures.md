---
id: TASK-0252
title: >-
  Count (or consciously exempt) resolve_chain read-error swallow sites in
  db_read_failures
status: To Do
assignee: []
created_date: '2026-07-08 17:22'
labels:
  - robustness
  - observability
dependencies:
  - TASK-0105
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0105 review (mped-architect MEDIUM finding). task-0105 added db_read_failures but only wired record_db_read_failure() at the two cache-lookup ENTRY POINTS (lookup_cached / lookup_cached_readonly). The SAME "real DB read error masquerades as a miss and wastes API quota" bug class exists, UNCOUNTED, at:
- whereamid/src/resolver.rs ~305 (resolve_chain skip_cached short-circuit): `Err(e) => { warn!("db error looking up {bssid}: {e}"); None }` — after swallowing to None the chain FALLS THROUGH and CALLS the providers (Apple/WiGLE), directly burning quota. Arguably the MORE damaging instance.
- whereamid/src/resolver.rs ~322-325 (resolve_chain skip_not_found short-circuit): `.is_not_found(...).await.unwrap_or(false)` — a DB read error is swallowed to false ("not in not_found") -> chain proceeds to call providers. Same harm.
Lower-relevance cousins to consider: server.rs ~2451 debug_statuses().unwrap_or_default() (mislabels DB status on the debug surface) and server.rs ~3063 state.db.stats().await.unwrap_or_default() (a read error in the stats handler SILENTLY ZEROES cached_aps/pending_aps/not_found_aps/db_size_bytes — the row counts lie while db_read_failures stays 0).
Consequence: db_read_failures is currently "cache-lookup-entry-point read failures only", NOT "all swallowed DB read failures"; ops could see db_read_failures:0 during an active DB fault on the resolve path. Fix direction: route the two resolve_chain sites through state.record_db_read_failure() (and decide on the stats.unwrap_or_default row-count lie), OR consciously document them as distinct and out of scope. Update the record_db_read_failure() doc-comment accordingly when done. Deterministic to test (DROP TABLE aps / not_found and drive resolve_chain).
<!-- SECTION:DESCRIPTION:END -->
