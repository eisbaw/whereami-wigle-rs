---
id: TASK-0231
title: >-
  Make the client sigpipe test hermetic: it sends a version request to the live
  daemon on 4747
status: To Do
assignee: []
created_date: '2026-07-02 22:42'
labels:
  - testing
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
QA gate finding (cycle: shutdown batch 0190/0102/0195, out of that range, introduced by task-0199): whereami-client/tests/sigpipe.rs json_write_to_closed_pipe_dies_by_sigpipe_not_panic runs `whereami --json version`, whose raw_command connects to the default 127.0.0.1:4747 and sends a version request to whatever is listening — on this host, the PRODUCTION daemon. The request is read-only and burns no quota, but the test suite must not depend on or touch a live daemon: `just e2e` is not hermetic and would behave differently on hosts with/without a daemon.

Fix direction from QA review: point the spawned client at a known-dead ephemeral port (bind a listener, note the port, drop it) — the task-0183 synthesized-JSON transport-error path still writes to stdout, which is all the EPIPE/SIGPIPE assertion needs; the test comment itself says it does not need a live daemon.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 the sigpipe test passes on a host with NO daemon listening (no dependency on a live daemon at 4747)
- [ ] #2 no test in the workspace connects to 127.0.0.1:4747
<!-- AC:END -->
