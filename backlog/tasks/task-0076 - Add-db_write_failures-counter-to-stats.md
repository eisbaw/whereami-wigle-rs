---
id: TASK-0076
title: Add db_write_failures counter to stats
status: To Do
assignee: []
created_date: '2026-05-10 11:01'
labels:
  - observability
  - server
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
23 'best-effort write failed' warn! sites silently allow DB writes to fail while logs are the only record. If last_fix or fixes ever become load-bearing (or storage fills up) operators have no quick way to ask 'is the DB writable?'. Expose db_write_failures_total via stats. Found in v0.4.0 review (keeper).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 DaemonState carries an AtomicU64 db_write_failures_total
- [ ] #2 Every warn-and-continue DB write site bumps the counter
- [ ] #3 stats response includes db_write_failures_total
<!-- AC:END -->
