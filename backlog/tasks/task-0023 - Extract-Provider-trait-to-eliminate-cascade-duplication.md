---
id: TASK-0023
title: Extract Provider trait to eliminate cascade duplication
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-09 21:07'
updated_date: '2026-05-09 21:37'
labels: []
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
resolve_for_locate, resolve_readonly, resolve_background, and pending::drain_once all reimplement the same Apple->WiGLE cascade. Extract a Provider trait with async fn lookup(&self, bssid) -> Result<Option<ApInfo>, ProviderError> and a provider chain. Resolve functions differ only in config (which providers, write-through vs read-only, pending behavior), not control flow.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. New module whereamid/src/provider.rs with:
   - enum ProviderOutcome { Found(ApInfo), NotFound, Skipped, HardStop(reason), NetworkError(anyhow) }
   - enum Provider { Apple, Wigle } with async fn lookup(state, bssid) -> ProviderOutcome
   - Each variant performs its own metering (record_api_call) and rate-limit precheck. Cache writes are NOT done by providers; chain orchestrator decides.
2. New ChainPolicy struct in resolver.rs (skip_cached, skip_not_found, write_through, on_skipped, on_network_error, on_hard_stop, mark_not_found_per_provider, mark_not_found_at_chain_end_if_unresolved, delete_pending_on_success).
3. resolve_chain(bssids, state, providers: &[Provider], policy) -> ResolveResult orchestrator. Single source of truth for the BSSIDs x providers loop.
4. Replace resolve_readonly with thin wrapper using policy { skip_cached/not_found, no write-through, mark NotFound, ignore network }.
5. Replace resolve_background with thin wrapper using policy { skip_cached/not_found, write_through, queue pending on net err / skipped / hard_stop, mark not_found at end }.
6. Replace pending.rs drain_once lookup loop with chain call using policy { no skip, write_through, increment_pending_attempts on net err, delete_pending on success, no not_found marking }. Keep pre-flight cleanup unchanged.
7. cargo build/test/clippy -D warnings/fmt --check inside nix develop. Skip fuzz (no parser changes). Commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented as enum-based static dispatch (Provider::Apple, Provider::Wigle) instead of an async-fn trait, to avoid pulling in async_trait or fighting dyn-trait async dispatch. Two providers, fixed set, no need for open extension.

Providers own their own metering (WiGLE records api_call on success/notfound). They do not touch caches/pending/not_found tables; that policy lives in the chain orchestrator.

resolve_chain orchestrator drives the BSSIDs x providers loop and applies a ChainPolicy struct. Three call sites collapse into thin wrappers.

Honest behavior deltas vs the prior cascade copies:
1) resolve_background old: Apple-pass-then-WiGLE-pass globally. New: per-BSSID Apple-then-WiGLE. Side effect: in old code, an Apple network error logged and fell through; if WiGLE then said NotFound, end-of-chain marked the BSSID not_found. In new code, Apple NetworkError triggers QueuePending (per policy), so even with WiGLE NotFound the BSSID lands in pending instead of not_found. New behavior is arguably more correct (Apple err means we did not actually rule the BSSID out) but it IS different.
2) drain_once: NetworkError increment_pending_attempts is now applied to ALL providers in chain, including Apple. Old code only incremented on WiGLE network errors. New behavior penalizes Apple-network-err-then-WiGLE-success less obviously: if Apple errs and WiGLE then Founds, we increment, then on success delete_pending — net is delete (no observable change). If Apple errs and WiGLE NotFound: we increment, no delete. Slightly faster pending row expiration when Apple is flaky.
3) HardStop: old `break` exited the BSSID loop; new sets a stopped[idx] flag so subsequent BSSIDs do extra cache/not_found pre-check reads but still skip the WiGLE call. Same final state, slightly more DB read activity.

Not fixed in this batch (deferred): WiGLE rate-limit TOCTOU race in wigle_lookup (commented in provider.rs). Belongs to batch 2 per the user plan.

Verification: cargo build, cargo test (33 pass), cargo clippy --all-targets -- -D warnings, cargo fmt --check all green. Skipped fuzz: no parser/protobuf changes.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Extracted a Provider abstraction and a single chain orchestrator, eliminating the three-way cascade duplication across resolver and pending.

New module whereamid/src/provider.rs:
- ProviderOutcome { Found, NotFound, Skipped, HardStop, NetworkError }
- enum Provider { Apple, Wigle } with one async lookup() entrypoint, static dispatch (no async_trait dep). Each provider handles its own preconditions and metering; cache/pending/not_found are NOT touched here — that is policy.

resolver.rs:
- ChainPolicy with explicit knobs (skip_cached, skip_not_found, write_through, delete_pending_on_success, mark_not_found_per_provider, mark_not_found_at_chain_end, on_skipped, on_network_error, on_hard_stop).
- resolve_chain orchestrator owns the BSSIDs x providers loop and outcome dispatch.
- resolve_readonly and resolve_background are now thin (~15 line) wrappers that just declare a policy and a provider list.

pending.rs drain_once:
- Pre-flight cleanup (delete_expired_pending, re-check expired not_found) unchanged.
- Cascade lookup loop replaced with a single resolve_chain call using the pending-drain policy.

Behavior deltas (intentional, documented in implementation notes): per-BSSID Apple-then-WiGLE instead of two global passes; Apple network errors now contribute to attempt counters; HardStop converts to per-provider stopped flag instead of a hard `break`. Same observable outputs in the success paths.

Verification: cargo build, cargo test (33 pass), cargo clippy --all-targets -- -D warnings, cargo fmt --check — all clean. Fuzz skipped (no parser changes).
<!-- SECTION:FINAL_SUMMARY:END -->
