---
id: TASK-0199
title: whereami | head panics with exit 101 (no broken-pipe handling)
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-07-02 04:52'
updated_date: '2026-07-02 17:27'
labels:
  - cli
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Rust ignores SIGPIPE before main, so once a downstream (head -1) exits, the next println! aborts: 'failed printing to stdout: Broken pipe', exit 101. All multi-line renderers (scan, debug, history) and early-closed --json | jq -e hit this; nothing resets SIGPIPE or catches the write error. The composable-CLI story the README leads with produces panic spew under ordinary pipelines. Ref whereami-client/src/main.rs:215-226,253-265,296-308.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 whereami scan | head -1 and whereami --json debug | head -c 10 exit without a panic message
- [x] #2 exit status under early-closed pipes is 0 (or a documented code), never 101 (reset SIGPIPE to SIG_DFL or map BrokenPipe to clean exit)
- [x] #3 behavior covered by a test or documented in CLI help
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Cargo.toml: add libc = 0.2 to [dependencies].
2. main.rs: at top of main() reset SIGPIPE to SIG_DFL via libc::signal with a comment; document the die-on-EPIPE (status 141) filter behavior in clap after_help.
3. tests/sigpipe.rs (#[cfg(unix)]): libc::pipe, close read end, spawn CARGO_BIN_EXE_whereami --json version with stdout=write end; assert status.signal()==SIGPIPE and stderr has no panicked. Depends on 0183.
4. Run test >=5x for flakiness; if flaky, delete test and keep help-doc route. Run gate, commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented SIGPIPE reset.
- main(): #[cfg(unix)] unsafe libc::signal(SIGPIPE, SIG_DFL) before any I/O. EPIPE now -> SIGPIPE death (status 141) like cat/grep; small outputs that fit the 64KB pipe buffer finish with exit 0. Never 101.
- after_help documents the filter/SIGPIPE(141) behavior in `whereami --help` (AC#3).
- libc = 0.2 added to [dependencies]; integration tests can use it directly (normal deps are visible to tests/, no dev-dependency needed).
- tests/sigpipe.rs closes the pipe read end pre-spawn so the child's first stdout write EPIPEs; asserts status.signal()==SIGPIPE and no "panicked". Deterministic because the read end is gone before the child writes. Depends on task-0183 (daemon-down --json version writes one synthesized line).
Flakiness verdict: NOT flaky. Ran the compiled test binary 40x (daemon up) = 40/40 pass; also passes in full `just e2e`. Kept the test.
Note: `scan | head -1` exits 0 (not 141) here because scan output is tiny and fully buffered before head closes — this is correct Unix-filter behavior (same as `cat smallfile | head`) and AC#2 explicitly allows 0. The 141/SIGPIPE path is exercised deterministically by the integration test.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Reset SIGPIPE to SIG_DFL at startup so `whereami` behaves like a standard Unix filter under pipes instead of panicking.

Why: the Rust runtime sets SIGPIPE to SIG_IGN before main(), so when a downstream reader closes the pipe (e.g. `whereami scan | head`) the next println! hits EPIPE and panics with "failed printing to stdout: Broken pipe" and exit 101 — panic spew under the very pipelines the README advertises.

Changes:
- main(): #[cfg(unix)] libc::signal(SIGPIPE, SIG_DFL) before any I/O. An early-closed pipe now terminates via SIGPIPE (status 141) like cat/grep; outputs that fit the pipe buffer still exit 0. Never 101.
- Documented the behavior in `whereami --help` (clap after_help).
- Added libc = "0.2".
- tests/sigpipe.rs: spawns the binary with stdout on a pipe whose read end is pre-closed, and asserts the process dies by SIGPIPE (not a panic/101).

User impact: `whereami ... | head` (and any early-closed pipe) no longer prints a Rust panic; exit is 141 (or 0), never 101.

Tests/verification: new integration test asserts signal()==SIGPIPE and no panic on stderr; ran it 40x with no flakiness; full `just e2e` green (clippy -D warnings + fmt clean). Verified live that the two AC pipelines exit without any panic message.
<!-- SECTION:FINAL_SUMMARY:END -->
