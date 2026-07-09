---
id: TASK-0108
title: 'Provider enum #[cfg(test)] Mock variant lives in production code'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:12'
updated_date: '2026-07-09 08:44'
labels:
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/provider.rs:42-70. The Provider::lookup match has a #[cfg(test)] arm for Mock. Production binaries compile with 2 variants, cargo test with 3. Has implications for any future #[derive(Serialize)] or exhaustive-match downstream — release-vs-test shape divergence is a smell. Partially addressed by task-0023's mention but not done.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Factor Provider into a trait (trait Provider { async fn lookup(...) -> ProviderOutcome; })
- [x] #2 Production impls: AppleProvider, WigleProvider
- [x] #3 Test-only MockProvider implementation in a tests/ module or shared test crate
- [x] #4 Resolver chain takes Vec<Box<dyn Provider>> or &[&dyn Provider]
- [x] #5 All existing resolver tests still pass; the chain no longer requires #[cfg(test)] in production sources
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add async-trait dep to whereamid/Cargo.toml (whereamid-only)
2. provider.rs: replace enum+impl with #[async_trait] trait Provider + AppleProvider/WigleProvider unit-struct impls; move MockProvider inherent lookup into a #[cfg(test)] trait impl; rewrite stale static-dispatch doc
3. resolver.rs: resolve_chain/_with_observations/_inner param &[Provider] -> &[&dyn Provider]; import AppleProvider/WigleProvider; update readonly (&[&WigleProvider]) and background (&[&AppleProvider,&WigleProvider]) call sites preserving order
4. pending.rs: drain_once param + drain loop call site; test helper mock() returns MockProvider
5. Update ALL test Mock constructions (Provider::Mock(...) -> &dyn Provider trait-object), keep Arc only in the multi-thread spawn test; assertions unchanged
6. Verify: nix develop -c just e2e, cargo fmt --check, grep confirms no cfg(test) in dispatch + enum gone + Cargo.lock only adds async-trait
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Triaged 2026-07-08 (marathon session, NOT started — deferred to a fresh context): the ACs are a DEEP refactor, not the minimal "hide the cfg(test) Mock variant" the title suggests — AC#1 trait-ify Provider (trait Provider { async fn lookup }), AC#2 AppleProvider/WigleProvider impls, AC#3 test-only MockProvider, AC#4 change resolve_chain to take Vec<Box<dyn Provider>> / &[&dyn Provider], AC#5 all resolver tests pass. That touches the provider abstraction + EVERY resolve_chain call site (server.rs, pending.rs, resolver tests) — high blast radius. Per phase3 loop stop-condition, a deep/high-regression-risk refactor warrants a FRESH context and should not be attempted fatigued at the end of a long marathon session. Pick this up first in a new session. NOTE: async-fn-in-trait needs either the (now-stable) native async fn in traits or async-trait; the enum currently dispatches statically, the trait version boxes — confirm no hot-path perf concern (providers are network-bound, so dyn dispatch is negligible).

Implemented 2026-07-09: enum -> #[async_trait] trait Provider dispatched as &dyn Provider.
- provider.rs: trait + AppleProvider/WigleProvider unit-struct impls + #[cfg(test)] impl Provider for MockProvider (moved inherent lookup into trait impl, ignores state). Rewrote stale static-dispatch doc.
- resolver.rs/pending.rs: resolve_chain/_with_observations/_inner + drain_once take &[&dyn Provider]. Production call sites: readonly=&[&WigleProvider], background/drain=&[&AppleProvider,&WigleProvider] (order preserved). No #[cfg(test)] in provider dispatch; enum deleted.
- Tests: only Mock CONSTRUCTION changed (Provider::Mock(...) -> trait objects); Arc kept only in the multi-thread spawn test. No assertion changed.
- Dep: async-trait 0.1.89 added to whereamid/Cargo.toml only; Cargo.lock adds only that package.
- Gate: nix develop -c just e2e -> 902 passed / 0 failed, clippy -D warnings clean; cargo fmt --check clean.
- Commit d0f8a92. Left In Progress for review; ACs unchecked.

## Review gate (orchestrator, independent) — GO/GO, behavior-identical
- QA (2x): 902/902 (count-neutral), zero flake; clippy 0 -D warnings; fmt clean. Behavior-preservation suites all green (resolve 30, chain 16, drain 20, pending 17, provider 12, not_found 29). Cargo.lock adds ONLY async-trait 0.1.89 (no version drift).
- Architecture (rigorous behavior-identity): NO test ASSERTION changed — grep of the full 161a16b..d0f8a92 diff for assert/prop_assert/unreachable/expect found only Mock CONSTRUCTION changes (Provider::Mock(Arc::clone(&m)) -> &mock / &dyn Provider) + one MOVED (not modified) call_count line; every outcome (NotFound/Skipped/HardStop/NetworkError/Found), unreachable guard, assert_eq value byte-identical. Provider ORDER preserved at all 3 production sites (pending drain + resolve_background = [&AppleProvider,&WigleProvider]; resolve_readonly = [&WigleProvider] wigle-only). Each trait impl dispatches identically to its old enum arm (apple_lookup/wigle_lookup; MockProvider::lookup ignores state, same as the old Mock arm). resolve_chain_inner byte-identical (signature-only &[Provider]->&[&dyn Provider]). Enum deleted; grep confirms NO #[cfg(test)] in AppleProvider/WigleProvider dispatch (only on MockProvider struct/impl + mod tests); ProviderOutcome untouched. async-trait whereamid-only; stale "static dispatch to avoid boxing" doc replaced with the boxed-future-is-negligible (network-bound) rationale. Arc retained ONLY in the multi-thread spawn test. No HIGH/MEDIUM/LOW findings.
- DESIGN NOTE: the enum was a deliberate choice to avoid async_trait/boxing (provider.rs:13), but that cost is negligible for NETWORK-bound provider lookups (nanoseconds vs a ms HTTP round-trip) — so the trait-ification is a net win (removes the release-vs-test shape divergence + eases adding providers) without meaningful overhead.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Factored the Provider enum (Apple/Wigle/#[cfg(test)]Mock) into an async trait, removing the release-vs-test shape divergence (production compiled 2 variants, tests 3) from production sources.

- trait Provider: Send + Sync { fn name() -> &static str; async fn lookup(&self, state, bssid) -> ProviderOutcome } (#[async_trait]).
- Production unit-struct impls AppleProvider/WigleProvider (dispatch to apple_lookup/wigle_lookup via state); #[cfg(test)] impl Provider for MockProvider. Enum deleted; no #[cfg(test)] remains in provider dispatch.
- resolver chain (resolve_chain/_with_observations/_inner + pending::drain_once) now takes providers: &[&dyn Provider]. Production call sites preserve order: pending drain + resolve_background = [apple, wigle]; resolve_readonly = [wigle] only.
- async-trait 0.1 added to whereamid only (Cargo.lock adds only async-trait; the enum\x27s "avoid boxing" rationale is moot — provider lookups are network-bound, so per-call boxing is negligible; the stale doc was replaced accordingly).

BEHAVIOR-IDENTICAL: reviewer grep-verified NO test assertion changed (only Mock construction), provider order preserved at every site, each impl dispatches identically to its old enum arm, resolve_chain_inner unchanged. 902 workspace tests (count-neutral), dual gate GO/GO, zero flake. Commit d0f8a92.
<!-- SECTION:FINAL_SUMMARY:END -->
