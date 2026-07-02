---
id: TASK-0194
title: resolve response reports not_found for merely skipped/deferred BSSIDs
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-07-02 04:51'
updated_date: '2026-07-02 18:18'
labels:
  - correctness
  - protocol
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Provenance::NotFound is documented as definitively absent, but handle_resolve emits it for every BSSID missing from result.positioned. That bucket includes BSSIDs deferred because another task holds the in-flight claim, and BSSIDs whose only provider (WiGLE via resolve_readonly) returned Skipped because creds are missing or quota is exhausted - the daemon contacted no provider yet tells the client the BSSID is definitively unknown. Wrong on the wire under common conditions; clients that cache not_found blacklist resolvable BSSIDs. Ref whereamid/src/server.rs:698-723,237-244, resolver.rs:235-242,297-304.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 a resolve for an uncached BSSID while WiGLE is unconfigured or quota-exhausted does not return source:not_found for it
- [x] #2 a resolve racing a background resolution of the same BSSID returns an in-progress/unresolved indication rather than not_found
- [x] #3 not_found on the wire is emitted only when a provider or the not_found table authoritatively reported absence; docs/protocol.md matches
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. resolver ResolveResult: add not_found: HashSet<String> = per-call authoritative absence (skip_not_found table short-circuit + chain-end definitive judgment). Document meaning.
2. resolve_chain: populate result not_found at short-circuit site and chain-end (definitive = authoritative_nf && !hard_transient, all policies incl Never); table write still gated to AtChainEnd.
3. daemon Provenance: add Unresolved (snake_case). handle_resolve: positioned->api/cache; result.not_found->not_found; else->unresolved.
4. client lib: add Unresolved variant + Display + wire tests. Keep #[serde(other)] Unknown.
5. protocol.md: document api|cache|not_found|unresolved; define not_found=authoritative absence, unresolved=daemon could not consult (deferred/skipped/transient).
6. Tests: unconfigured/quota Skipped->not not_found (AC1); inflight race->not in result.not_found (AC2); mapping fn/handler test; wire round-trips (AC3).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented in commit accf3b7 (resolver.rs, server.rs, whereami-client/src/lib.rs, docs/protocol.md).
- resolver ResolveResult gains not_found: HashSet<String> = per-call authoritative absences (not_found-table short-circuit within TTL + chain-end definitive verdict). Classification runs for ALL policies incl Never so resolve_readonly reports honest not_found without persisting (independent of table-write, per task-0092).
- New daemon Provenance::Unresolved (serde snake_case). handle_resolve: positioned->api/cache; result.not_found->not_found; else->unresolved (one value for deferred/skipped/transient; identical client semantics: absence not established, retry).
- Client Provenance::Unresolved + Display + wire round-trip. Older clients map via #[serde(other)]->Unknown.
- protocol.md: source vocab now api|cache|not_found|unresolved with definitions; versioning note (new enum values additive, v stays 1).
Deviation from brief: did NOT add a separate ResolveResult.deferred field. handle_resolve subsumes deferred into the else->unresolved bucket, so a deferred field would be unused wire-facing state (YAGNI / minimize-state). AC#2 is verified via the inflight-claim path instead. Renamed server test module history_dispatch_tests -> dispatch_tests to reuse its test_state (SSOT) rather than duplicate it a 4th time.
Tests: resolver resolve_chain_not_found_set_is_authoritative_only + augmented inflight-dedup test (deferred not in not_found); server dispatch_tests resolve_uncached_wigle_unconfigured_is_unresolved (AC1), resolve_racing_inflight_claim_is_unresolved (AC2), resolve_table_not_found_is_reported_as_not_found (AC3 positive); daemon+client provenance wire round-trips include unresolved.
Gate: whereamid lib 78 / bin 128 / proptests 11; client bins 7/22/1; clippy -D warnings clean; cargo fmt --check clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
handle_resolve reported Provenance::NotFound for every BSSID missing from positioned, including ones deferred to an in-flight resolution and ones whose only provider (WiGLE) was Skipped for creds/quota — telling clients a BSSID was definitively absent when no provider was consulted.

Changes:
- resolver ResolveResult gains not_found: HashSet carrying per-call authoritative absences (not_found table within TTL + chain-end definitive verdict). Classification runs for every policy so it stays honest under resolve_readonly (Never), independent of table writes (task-0092).
- New daemon Provenance::Unresolved (serde "unresolved"). handle_resolve maps positioned->api/cache, result.not_found->not_found, else->unresolved (one value for deferred/skipped/transient: identical client semantics).
- whereami-client: matching Unresolved variant + Display + wire round-trip; older clients map via #[serde(other)].
- docs/protocol.md: source vocab api|cache|not_found|unresolved with definitions + versioning note.

Tests: resolver not_found-set is authoritative-only and empty for a deferred BSSID; dispatch tests prove unconfigured-WiGLE (AC#1) and in-flight race (AC#2) yield unresolved while a table not_found yields not_found (AC#3); daemon+client wire round-trips include unresolved.

Deviation: omitted a separate ResolveResult.deferred field (handle_resolve subsumes deferred into unresolved; adding it would be unused state).
<!-- SECTION:FINAL_SUMMARY:END -->
