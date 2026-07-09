---
id: TASK-0276
title: >-
  Test the accept-loop permit-acquire cooperative-shutdown branch (saturated
  semaphore)
status: To Do
assignee: []
created_date: '2026-07-09 08:54'
labels:
  - test
  - shutdown
  - concurrency
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0143 review LOW-1: run_server's accept loop has TWO per-iteration await points, each with a biased state.shutdown.cancelled() break branch. The ACCEPT-select branch is exercised by sigterm_exits_within_drain_budget (idle daemon parks there). The PERMIT-ACQUIRE branch (server.rs ~907-913) — the 'shutdown while parked waiting for a connection slot with the semaphore exhausted' scenario — is verified only by inspection (structurally identical biased+cancel+break), NOT by a test. A regression that dropped the permit-branch cancel would not be caught by the idle test. Add a test that saturates the connection-admission semaphore (hold the max in-flight connections open so acquire_owned parks), then SIGTERM, asserting the daemon still exits cooperatively within the drain budget.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A shutdown test holds enough concurrent connections to exhaust the connection-admission semaphore so the accept loop is parked on acquire_owned(), then sends SIGTERM and asserts cooperative clean exit within the drain budget (the cooperative-exit log present, no force-abort warning)
<!-- AC:END -->
