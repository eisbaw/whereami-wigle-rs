---
id: TASK-0193
title: Bound and dedupe background Nominatim geocode tasks
status: Done
assignee:
  - '@myself'
created_date: '2026-07-02 04:51'
updated_date: '2026-07-05 00:07'
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
- [x] #1 N concurrent/rapid locates at one uncached position trigger exactly one Nominatim request (HashSet of in-flight grid keys, like the resolver InflightGuard)
- [x] #2 with Nominatim unreachable the number of live geocode tasks stays bounded regardless of locate rate
- [x] #3 requests are not issued for positions superseded more than a bounded interval ago
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a server/geocode scheduler with one named worker, an exact rounded-grid HashSet claim, and a latest-only pending slot; every address-enabled locate observes its current key, including cache hits, so newer positions supersede older work.
2. Move the request seam inside Nominatim below its shared one-second rate gate. Dispatch requires the candidate generation to remain current and queue age to be within REQUEST_INTERVAL plus one second scheduling tolerance (currently two seconds); response handling requires only that the generation remain latest. Already-issued HTTP cannot be recalled.
3. Preserve successful cache insertion and TASK-0046 last_fix -> db backfill ordering; own the worker in the named shutdown drain and use RAII cleanup for success, error, supersession, cancellation, panic, and forced abort.
4. Add deterministic injected-transport tests for active same-cell dedup, raw-coordinate jitter, thousands of moving/unreachable submissions with hard 1-worker/1-pending bounds, ordered publication/latest-wins, rate-slot and queue-age boundaries, slow responses, retry, shutdown/panic cleanup, and stale-result non-mutation.
5. Run focused/stress tests, just e2e, fmt, and diff checks; then require parallel qa-test-runner and mped-architect GO before checking ACs, marking Done, and committing.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Phase-3 contract (2026-07-05): AC#3 is enforced at outbound transport invocation. Superseded generations are rejected immediately. Current work may wait one shared Nominatim REQUEST_INTERVAL plus one second of explicit scheduling tolerance (two seconds total) before dispatch; after transport is invoked, normal network latency does not invalidate a result that remains the latest generation. An already-issued HTTP request cannot be unsent, but its response is discarded if superseded.

Implementation: one named GeocodeScheduler worker owns all rate waiting and HTTP work; locates never spawn geocode tasks. Scheduler state has one exact rounded-grid active HashSet claim, one latest-only pending Candidate, ordered generation publication under the state mutex, and RAII cleanup. Cache hits and misses both publish current position. Nominatim transport injection sits below the real rate gate. Locate linearization installs last_fix, atomically observes cache -> scheduler, releases cache, then persists under the retained last_fix guard for TASK-0046.

Review remediation: the first mped-architect/qa gate found re-orderable watch publication, empty rate-slot consumption, incomplete panic cleanup, cache held across SQLite, and unsafe test joins. The second gate found queue age incorrectly reused after HTTP, no rate-gate scheduling tolerance, and a stale comment. Fixes serialize state+watch publication, make state authoritative, reject freshness before advancing the rate timestamp, separate dispatch freshness from latest-response validity, clear pending on every worker exit, release cache before DB, and make all test cleanup bounded and JoinError-aware.

Verification: 11 scheduler tests plus the Nominatim rate-slot test pass. Same-cell/cache-hit, delayed-notification, empty-slot, panic-cleanup, slow-current-response, and rate-tolerance interleavings each passed 20/20. Mutations produced duplicate requests, lost newest work, an extra virtual rate second, retained pending after panic, missed cache-hit cancellation, and discarded a slow current response. Restored `just e2e` passed 340 tests plus strict Clippy; `just fmt` and `git diff --check` passed. Final parallel gate: mped-architect GO and qa-test-runner GO; QA independently repeated all focused tests 6/6 and the full 340-test gate.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced unbounded per-locate Nominatim spawns with one named, shutdown-owned latest-only worker. Exact rounded-grid active claims dedupe same-cell requests; one pending slot bounds moving/unreachable workloads; ordered generations and cache-hit observations supersede stale work. Freshness is checked below the shared one-second rate gate with a two-second pre-dispatch budget, while slow responses remain valid only if still latest. RAII clears active/pending state across errors, shutdown, panic, and abort, and TASK-0046 last_fix -> DB ordering is preserved. Deterministic injected-transport tests cover all race, bound, retry, freshness, slow-response, and cleanup paths; final independent QA and architecture reviews returned GO.
<!-- SECTION:FINAL_SUMMARY:END -->
