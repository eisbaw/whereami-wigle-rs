---
id: TASK-0231
title: >-
  Make the client sigpipe test hermetic: it sends a version request to the live
  daemon on 4747
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-07-02 22:42'
updated_date: '2026-07-03 05:52'
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
- [x] #1 the sigpipe test passes on a host with NO daemon listening (no dependency on a live daemon at 4747)
- [x] #2 no test in the workspace connects to 127.0.0.1:4747
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Fixed in commit f9bae28 (rode along with the client transport rewrite for task-0196).

Approach: the sigpipe test now sets WHEREAMI_SOCKET to a guaranteed-absent unix path (unique under temp_dir, never created) and clears WHEREAMI_ADDR on the spawned child. WHEREAMI_SOCKET is resolved verbatim (no existence check), so `whereami --json version` connects to the absent socket -> ENOENT -> task-0183 synthesized-JSON transport error on stdout -> the very EPIPE/SIGPIPE the test asserts. It never touches any daemon.

NOTE: the brief said tempfile is a workspace dev-dep; it is NOT (only transitive in Cargo.lock). Followed the existing shutdown.rs hand-rolled-temp-path pattern instead of adding a dependency.

AC#2 verification (grep of test code): the only 4747 references in tests are shutdown.rs SAFETY doc + two assert_ne!(port, 4747) guards (never connect), config.rs --bind ARG VALUES (pure Args::parse_from, no network), and lib.rs resolve_target unit tests using a MOCK 10.0.0.5:4747 with an injected exists-closure (no network) plus the never_falls_back_to_tcp_4747 assertion. `grep TcpStream::connect|create_connection` across tests/: none. No test connects to 127.0.0.1:4747.

Gate: sigpipe test passes (1 passed) in the full e2e run.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Made the client sigpipe test hermetic (task-0231). It previously ran `whereami --json version`, whose default transport connected to 127.0.0.1:4747 and hit whatever was listening — on this host, the production daemon.

Fix (in commit f9bae28, alongside the task-0196 client transport rewrite): the spawned child gets WHEREAMI_SOCKET pointed at a guaranteed-absent unix path and WHEREAMI_ADDR cleared. WHEREAMI_SOCKET is used verbatim (no existence check), so the connect fails with ENOENT and the task-0183 synthesized-JSON path still writes one line to stdout — the EPIPE/SIGPIPE the test asserts — without touching any daemon. Behaves identically on hosts with or without a live daemon.

AC#1: verified the test passes in the e2e run and is daemon-independent by construction. AC#2: grep of all test code shows the only 4747 references are non-connecting (assert_ne guards, pure arg-parse values, a mock resolution addr, and a no-fallback assertion); no TcpStream::connect / create_connection to 4747 anywhere. The new default resolution is unix-only with no TCP fallback.

Note: tempfile was NOT actually a dev-dep (only transitive), so this used the existing shutdown.rs hand-rolled-temp-path pattern rather than adding a dependency.
<!-- SECTION:FINAL_SUMMARY:END -->
