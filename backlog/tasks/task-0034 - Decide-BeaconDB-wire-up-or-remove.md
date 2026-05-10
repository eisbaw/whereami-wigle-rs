---
id: TASK-0034
title: 'Decide BeaconDB: wire up or remove'
status: To Do
assignee: []
created_date: '2026-05-10 05:37'
labels:
  - decision
  - cleanup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
BeaconDbClient is constructed in main.rs and stored on DaemonState but never called. After task-0027 it is annotated #[allow(dead_code)] with a 'kept for future' comment, but that future has been pending since the original design. Either integrate it into resolve_chain as a third Provider variant (Apple → WiGLE → BeaconDB), or delete the module + field + construction entirely. Pick one.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Either: BeaconDB is wired as a Provider variant and exercised by an integration test, OR the module + DaemonState.beacondb + main.rs construction are all deleted
- [ ] #2 No #[allow(dead_code)] remaining for beacondb-related code
- [ ] #3 PRD updated to match the chosen direction
<!-- AC:END -->
