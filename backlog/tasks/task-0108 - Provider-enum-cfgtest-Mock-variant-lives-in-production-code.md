---
id: TASK-0108
title: 'Provider enum #[cfg(test)] Mock variant lives in production code'
status: To Do
assignee: []
created_date: '2026-05-20 21:12'
updated_date: '2026-07-08 18:59'
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
- [ ] #1 Factor Provider into a trait (trait Provider { async fn lookup(...) -> ProviderOutcome; })
- [ ] #2 Production impls: AppleProvider, WigleProvider
- [ ] #3 Test-only MockProvider implementation in a tests/ module or shared test crate
- [ ] #4 Resolver chain takes Vec<Box<dyn Provider>> or &[&dyn Provider]
- [ ] #5 All existing resolver tests still pass; the chain no longer requires #[cfg(test)] in production sources
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Triaged 2026-07-08 (marathon session, NOT started — deferred to a fresh context): the ACs are a DEEP refactor, not the minimal "hide the cfg(test) Mock variant" the title suggests — AC#1 trait-ify Provider (trait Provider { async fn lookup }), AC#2 AppleProvider/WigleProvider impls, AC#3 test-only MockProvider, AC#4 change resolve_chain to take Vec<Box<dyn Provider>> / &[&dyn Provider], AC#5 all resolver tests pass. That touches the provider abstraction + EVERY resolve_chain call site (server.rs, pending.rs, resolver tests) — high blast radius. Per phase3 loop stop-condition, a deep/high-regression-risk refactor warrants a FRESH context and should not be attempted fatigued at the end of a long marathon session. Pick this up first in a new session. NOTE: async-fn-in-trait needs either the (now-stable) native async fn in traits or async-trait; the enum currently dispatches statically, the trait version boxes — confirm no hot-path perf concern (providers are network-bound, so dyn dispatch is negligible).
<!-- SECTION:NOTES:END -->
