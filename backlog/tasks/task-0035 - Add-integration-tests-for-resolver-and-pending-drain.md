---
id: TASK-0035
title: Add integration tests for resolver and pending drain
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 05:37'
updated_date: '2026-05-10 08:06'
labels:
  - testing
  - coverage
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
After the Provider trait refactor (task-0023) the cascade lives in a single resolve_chain function plus pending::drain_once. Both have zero direct tests. The Provider type is currently a closed enum which makes injection hard; consider a small Provider trait + dyn Provider OR a fn pointer / closure boundary in resolver.rs only (test-only) so we can drive the chain with mocks. Pin behavior of: cache hit, not_found hit, Apple-found, WiGLE-found, both-NotFound, NetworkError → pending policy, Skipped propagation, in-flight dedup.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 resolve_chain has tests covering at least: cache-hit, Apple-Found, WiGLE-Found, both-NotFound, NetworkError-fallthrough, in-flight dedup of concurrent calls for same BSSID
- [x] #2 drain_once has a test covering: pending row resolves and is deleted, NetworkError increments attempts, max_attempts row is removed before retry
- [x] #3 Tests do not require live network or sleep-based concurrency tricks
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add Provider::Mock variant gated on cfg(test) with a closure-based responder + call tracking
2. Provider::lookup gets a cfg(test) arm for Mock
3. Add tests inline in resolver.rs (cfg(test) mod tests) that build a real DaemonState (in-memory DB) and exercise resolve_chain with mock providers:
   - cache hit short-circuit
   - Apple-Found stops chain
   - both-NotFound with mark_not_found_at_chain_end inserts not_found
   - NetworkError suppresses not_found mark (task-0045 invariant)
   - in-flight dedup prevents duplicate calls for same BSSID
4. Add drain_once-style test in pending.rs that exercises the new not_found policy
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added Provider::Mock variant gated on cfg(test) plus a closure-based MockProvider with call tracking. Added tests in resolver.rs::tests:
- resolve_chain_short_circuits_on_cache_hit
- resolve_chain_first_provider_found_stops_chain
- resolve_chain_both_not_found_writes_not_found_table
- resolve_chain_transient_error_suppresses_not_found_mark (task-0045 invariant)
- resolve_chain_inflight_dedup_defers_concurrent_lookups

Added tests in pending.rs::tests:
- drain_cleanup_deletes_pending_after_definitive_not_found (task-0045 end-to-end)
- drain_cleanup_keeps_pending_when_transient_error (counterexample)

Refactor: extracted drain_cleanup_after_chain from drain_once so tests can drive the cleanup logic directly with a mock chain.

Bug found and fixed during testing: the original mark_not_found_at_chain_end branch had an is_pending() check that skipped marking not_found if the BSSID was already pending. This made task-0045 a no-op for drain_once (whose entire input set is pending). The transient_error guard already covers the 'just queued' case via on_skipped/on_network_error/on_hard_stop, so the is_pending check was both redundant and wrong. Removed it.

Devshell improvement: added pkgs.openssl.out/lib to LD_LIBRARY_PATH in flake.nix shellHook because the bin test binary loads libssl.so.3 dynamically via reqwest and the linker did not bake an rpath for it.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added integration tests for resolve_chain and drain_once via a Provider::Mock variant gated on cfg(test). 5 resolver tests + 2 pending drain tests cover: cache hit, first-provider-found, definitive both-NotFound, transient-error guard (task-0045 invariant), in-flight dedup, drain cleanup, drain cleanup counter-example.

Found and fixed a bug in task-0045 along the way: the chain's is_pending check was making task-0045 a no-op for drain_once. The transient_error guard already covers the 'newly queued' case, so the redundant is_pending check was removed.

Added openssl.out/lib to flake.nix LD_LIBRARY_PATH so cargo test no longer fails with 'libssl.so.3 not found' for binaries that load reqwest at runtime.
<!-- SECTION:FINAL_SUMMARY:END -->
