---
id: TASK-0184
title: parse_range panics on chrono overflow (reachable from any TCP history request)
status: To Do
assignee: []
created_date: '2026-07-02 04:49'
labels:
  - correctness
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
parse_range parses the count as any positive i64 and feeds it into panicking chrono APIs: Duration::days(n) etc panic above TimeDelta bounds, and now - dur panics when the year underflows (~262144). {"cmd":"history","range":"100000000d"} (10 bytes, well inside the 64 KiB request cap) deterministically panics the connection task; the client gets a closed socket instead of the protocol-mandated error JSON. Also crashes whereami history --range typos. The uncommitted Kani harness parse_range_no_panic (32-byte domain) is a live counterexample - the harness is correct, production is not. Ref whereamid/src/history.rs:139-158, server.rs:919.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 parse_range returns Err (never panics) for any i64 magnitude and any unit; regression tests cover both overflow sites (TimeDelta construction and DateTime subtraction)
- [ ] #2 a TCP history request with a huge range yields an ok:false error response and the connection survives
- [ ] #3 the Kani parse_range_no_panic harness passes against the fixed code
<!-- AC:END -->
