---
id: TASK-0035
title: Add integration tests for resolver and pending drain
status: To Do
assignee: []
created_date: '2026-05-10 05:37'
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
- [ ] #1 resolve_chain has tests covering at least: cache-hit, Apple-Found, WiGLE-Found, both-NotFound, NetworkError-fallthrough, in-flight dedup of concurrent calls for same BSSID
- [ ] #2 drain_once has a test covering: pending row resolves and is deleted, NetworkError increments attempts, max_attempts row is removed before retry
- [ ] #3 Tests do not require live network or sleep-based concurrency tricks
<!-- AC:END -->
