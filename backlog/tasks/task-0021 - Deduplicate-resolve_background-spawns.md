---
id: TASK-0021
title: Deduplicate resolve_background spawns
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-09 21:07'
updated_date: '2026-05-09 21:56'
labels: []
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scan loop spawns resolve_background every tick. Locate fallback also spawns one for the same BSSIDs. Two concurrent tasks hit Apple/WiGLE simultaneously, doubling API consumption. Add a HashSet of currently-resolving BSSIDs behind a Mutex to prevent redundant work.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 DaemonState has inflight HashSet field
- [x] #2 resolve_chain skips BSSIDs already in inflight, owns insert/remove for the duration of provider work
- [x] #3 Cache-hit and not_found-skip paths do not interact with inflight (they don't make provider calls)
- [x] #4 Inflight entries are cleaned up on all exit paths
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add inflight: tokio::sync::Mutex<HashSet<String>> field to DaemonState. Initialize in main.rs.
2. In resolve_chain, for each BSSID: lock inflight, atomically check-and-insert; if was already present, skip (continue) — another task is handling it. The two concurrent spawns will not duplicate provider calls.
3. Use a scope-local Vec<String> of "we own these in-flight tags"; on every exit path of the loop body, remove from the inflight set. Implement with a small InflightGuard (Drop ergonomics) — but since async/Drop is awkward, just manage explicitly with a flag and remove at end of iteration.
4. Ensure cache-hit and not-found shortcircuits do NOT enter inflight (they don't need provider calls).
5. Add a unit test or integration test if feasible — likely difficult without mocking providers, so verify by reasoning + manual run if needed. Skip if no clean test boundary.
6. Build, clippy, fmt, test.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added inflight: std::sync::Mutex<HashSet<String>> to DaemonState. Used std (not tokio) Mutex because we never hold across await; this enables a Drop-based RAII guard.

New InflightGuard in resolver.rs claims a BSSID atomically (insert == was_absent) and releases on drop, so even a panic mid-resolution does not leak entries.

resolve_chain integrates the guard between the not_found shortcircuit and the providers loop. BSSIDs already in-flight are skipped (deferred_to_other), and excluded from the chain-end not_found marking so the owning task makes that decision.

No new automated test: faithfully exercising the dedup requires injectable providers (current Provider enum has hard-coded backends). Reasoning + manual verification: with two concurrent resolve_background calls on overlapping BSSIDs, the second one will see the first's claims and skip. Same for locate cold-start vs scan loop.

Limitations / honest concerns:
- Per-BSSID granularity, not per-batch: each spawn still iterates its full input list. Trivial overhead.
- Locate cold-start path discards the resolve_background return value already, so the deferred-no-result behavior is fine.
- If resolve_chain is called recursively (it is not currently) the guard would self-deadlock the second call by skipping its own work — non-issue today.

build/clippy/fmt/test all clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Coalesce concurrent BSSID lookups so scan loop, locate cold-start, and pending drain do not duplicate Apple/WiGLE traffic.

Changes:
- DaemonState gains `inflight: std::sync::Mutex<HashSet<String>>`. std::sync is fine here because we never hold the mutex across an await; this also lets us implement RAII cleanup.
- New `InflightGuard` in resolver.rs: `try_claim` does an atomic insert-if-absent; `Drop` removes the BSSID. Drop runs on the panic-unwinding path too, so a panic mid-resolution cannot leak entries and permanently block future lookups for that BSSID.
- `resolve_chain` claims each BSSID after the cache + not_found shortcircuits and before the provider walk. If another task already owns the BSSID we record it in `deferred_to_other` and skip. End-of-chain not_found marking now also skips deferred-to-other BSSIDs so the owning task is the sole decision maker.

Why: scan loop spawns resolve_background every tick, and locate cold-start spawns the same flow for the same APs. Without dedup both pass the cache miss together and both hit Apple+WiGLE. With pending drain in the mix the same BSSID can be in three concurrent flights, tripling external API consumption and quota burn.

Limitations (honest):
- No automated test. The Provider enum has hard-coded backends and no injection point; a meaningful test would require introducing a mock Provider variant just for this. Decided not to widen the production type for the test (MPED: dont add scaffolding without value). Manual reasoning + the `claimed = HashSet::insert` invariant carry the correctness argument.
- Dedup is per-BSSID, not per-batch. Each spawned resolve_chain still iterates its entire BSSID list; only the provider call (the expensive part) is coalesced.

Tests: cargo build, cargo clippy --all-targets -D warnings, cargo fmt --check, cargo test (25 unit + 9 proptest) all clean.
<!-- SECTION:FINAL_SUMMARY:END -->
