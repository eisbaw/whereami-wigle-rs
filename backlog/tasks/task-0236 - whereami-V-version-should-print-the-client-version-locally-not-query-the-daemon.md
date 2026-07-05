---
id: TASK-0236
title: >-
  whereami -V/--version should print the client version locally, not query the
  daemon
status: Done
assignee:
  - task0236-implementer
created_date: '2026-07-03 07:33'
updated_date: '2026-07-05 10:53'
labels:
  - client
  - ux
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
QA finding (cycle 9, task-0209 gate, L5): the client's -V/--version flag is documented in --help as 'equivalent to the version subcommand', which CONNECTS TO THE DAEMON. Convention for --version is a local, offline answer (print client version + embedded GIT_REV and exit 0). Surprising UX: whereami --version fails with a transport error when no daemon is reachable. Keep the version SUBCOMMAND daemon-querying (it reports daemon version/git_rev); make the -V/--version FLAG local-only. Sites: whereami-client/src/main.rs argv handling + help text; add a test that -V succeeds with WHEREAMI_SOCKET pointed at an absent path.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 -V and --version print the embedded client package version and GIT_REV and exit 0 without resolving or connecting to a daemon
- [x] #2 the version subcommand remains daemon-querying and continues to report both client and daemon versions
- [x] #3 --json --version is local-only and emits one valid JSON object with ok=true, version, and git_rev
- [x] #4 --help distinguishes the local flag from the daemon-querying version subcommand
- [x] #5 a real-process test points WHEREAMI_SOCKET at an absent path and proves both version flags still succeed without stderr
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Split the local version flag from the daemon-querying version subcommand.
2. Handle -V/--version immediately after clap parsing and before target resolution; print embedded client package version and GIT_REV.
3. Keep --json --version local and emit a valid synthesized JSON object with ok/version/git_rev; keep the ordinary flag banner human-readable.
4. Update help/comments and add real-process absent-socket coverage for -V, --version, and JSON mode; prove the version subcommand remains daemon-backed.
5. Run focused tests, mandatory parallel review, final E2E, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Policy chosen for the previously unspecified flag combination: --json --version remains offline and emits synthesized valid JSON containing ok=true, version, and git_rev. The version subcommand remains the daemon-querying protocol surface. Human -V/--version preserves the existing client banner line and exits 0 without resolving a socket.

Implementation: the version flag short-circuits immediately after successful clap parsing and before daemon target resolution or timeout configuration. Human output preserves the existing client banner; --json --version serializes a local ok/version/git_rev object. The daemon-backed handler is explicitly named run_daemon_version and remains the version subcommand path. Hermetic real-process tests cover -V, --version, --json --version, and the daemon-dependent subcommand against a guaranteed-absent socket.

Review: the first frozen behavior passed functional checks but architecture found stale JSON-contract documentation. Updated main comments and protocol.md to distinguish daemon passthrough/synthesized error JSON from the local version success object (no protocol v; key order non-contractual). Final combined tracked+untracked hash fdbf7d2a2fcfbb8b57ac8a9e1c37387fa2b23b8e5852caa2cc0af0aef0743688 received QA GO and architecture GO; 505 integrated tests and strict clippy passed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
The conventional -V/--version flags now report the embedded client version and Git revision offline with exit 0. JSON mode emits a documented local success object. The version subcommand remains daemon-querying, with hermetic process tests pinning all four paths.
<!-- SECTION:FINAL_SUMMARY:END -->
