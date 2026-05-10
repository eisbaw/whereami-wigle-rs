---
id: TASK-0074
title: Add inflight count to stats response
status: Done
assignee: []
created_date: '2026-05-10 11:00'
updated_date: '2026-05-10 14:10'
labels:
  - observability
  - server
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
DaemonState.inflight HashSet has no operator visibility. If a provider hangs despite timeouts, the set grows silently. stats handler (server.rs:870-888) already exposes cached/pending/not_found counts; add inflight_count for parity. Trivial change, big debugging payoff later. Found in v0.4.0 review (keeper).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 StatsResponse has inflight_count: usize
- [x] #2 whereami-client lib + CLI render the new field
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added inflight: usize to StatsResponse on both server and client sides. server.rs::handle_stats reads state.inflight.lock().len() (or e.into_inner().len() on poison). CLI renders 'inflight: N' in stats output.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
stats endpoint now exposes the in-flight provider-lookup set size. Helps spot stuck Apple/WiGLE lookups despite HTTP timeouts.
<!-- SECTION:FINAL_SUMMARY:END -->
