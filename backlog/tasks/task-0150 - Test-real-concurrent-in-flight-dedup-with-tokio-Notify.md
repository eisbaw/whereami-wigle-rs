---
id: TASK-0150
title: 'Test: real concurrent in-flight dedup with tokio::Notify'
status: To Do
assignee: []
created_date: '2026-05-20 21:29'
labels:
  - tests
  - concurrency
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P0-3. resolver.rs:643 pre-registers BSSID directly to simulate dedup; swapping guard.insert for guard.contains would never insert anything but the test still passes. No real concurrent test exists.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Test spawns two tokio::spawn calls to resolve_chain for the same BSSID where the MockProvider holds a tokio::sync::Notify and waits before returning
- [ ] #2 Assert mock.call_count() == 1 (one provider call, not two)
- [ ] #3 Assert the second (deferred) caller returns empty positioned with the right outcome
- [ ] #4 Assert the in-flight set is empty at end (RAII guard released)
- [ ] #5 Notify makes the test deterministic — no flakes
<!-- AC:END -->
