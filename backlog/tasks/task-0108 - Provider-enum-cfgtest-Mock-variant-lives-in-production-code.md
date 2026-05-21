---
id: TASK-0108
title: 'Provider enum #[cfg(test)] Mock variant lives in production code'
status: To Do
assignee: []
created_date: '2026-05-20 21:12'
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
