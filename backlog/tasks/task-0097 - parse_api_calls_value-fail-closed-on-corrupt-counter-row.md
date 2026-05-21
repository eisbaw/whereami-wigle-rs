---
id: TASK-0097
title: 'parse_api_calls_value: fail-closed on corrupt counter row'
status: To Do
assignee: []
created_date: '2026-05-20 21:10'
labels:
  - correctness
  - wigle
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/db.rs:52-66. parse_api_calls_value returns 0 when metadata.api_calls_today is non-numeric. Combined with the daemon's optimistic mirror of WiGLE's server-side counter, a corrupted row (disk gremlins, partial write) gets healed by the next try_reserve_api_call writing the bumped value over the garbage. Daemon then issues up to daily_limit fresh WiGLE calls today on top of whatever was already spent — silently double-spending an irreversible API budget. task-0077 added a warn; consequence is not contained.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 On parse failure, parse_api_calls_value (or its caller try_reserve_api_call) returns Err / treats the count as daily_limit (deny-all) rather than 0 (allow-all)
- [ ] #2 Provider treats the Err as ProviderOutcome::Skipped (no quota spent, no row corruption)
- [ ] #3 Unit test: corrupt metadata.api_calls_today to 'garbage', call try_reserve_api_call, assert it returns false and the row is not overwritten
- [ ] #4 tracing::warn fires once per corruption (already exists from task-0077); now paired with fail-closed behavior
<!-- AC:END -->
