---
id: TASK-0240
title: 'whereamid logs to stdout (tracing default), not stderr - decide/document'
status: Done
assignee:
  - '@myself'
created_date: '2026-07-04 17:33'
updated_date: '2026-07-09 05:48'
labels:
  - robustness
  - observability
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Discovered while implementing task-0233. tracing_subscriber::fmt() default MakeWriter is STDOUT, so whereamid writes all lifecycle INFO/WARN logs to stdout (confirmed empirically: with stdout->file and stderr->file, every INFO/WARN line lands on stdout; stderr receives only Rust panics and a main-returned Err). Unconventional for a daemon (logs usually go to stderr so stdout can carry data), though harmless under systemd (both -> journal). It also made tests/shutdown.rs read_stderr misleading: the task-0233 graceful-degrade test had to capture stdout (new read_output helper) to observe the degrade + readiness lines. Decide: either .with_writer(std::io::stderr) for convention, or document stdout-by-design. If changed, audit tests/shutdown.rs (read_output/read_stderr split) and any log consumer. Ref whereamid/src/main.rs tracing_subscriber init (~L45).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A decision is recorded: stderr for convention, OR documented stdout-by-design with rationale
- [x] #2 If changed to stderr: the subscriber uses .with_writer(std::io::stderr) and tests/consumers are updated; if kept on stdout: a comment at the subscriber init documents it
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add .with_writer(std::io::stderr) to the tracing_subscriber builder in whereamid/src/main.rs (only stream changes; content/levels unchanged).
2. Audit tests/shutdown.rs: lifecycle assertions use read_output (stdout+stderr combined) so they still see the lines now on stderr; fix the now-stale comments that claim logs go to stdout. live_socket_refused reads stderr for the main-returned Err (unchanged). Run shutdown tests.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DECISION: stderr (convention). Added .with_writer(std::io::stderr) to the tracing_subscriber::fmt() builder (main.rs:65) — only the stream changed, content/levels unchanged. Reviewer CONFIRMED the shutdown tests still observe the moved lines: read_output (shutdown.rs:84-93) concatenates stdout.log+stderr.log, so readiness/degrade/WiGLE-warn assertions still see the (now-stderr) lines; the two read_stderr uses are correct+unaffected (sigterm test asserts !panicked; live_socket test asserts the main-returned anyhow Err, which Rust prints to stderr, not a tracing log). NO assertion expected a stdout-specific line. Only stale doc-comments in shutdown.rs were corrected (no assertion changed). Dual gate GO/GO (876).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Moved whereamid lifecycle logs from stdout to stderr (convention: logs->stderr so stdout can carry data). One-line change: .with_writer(std::io::stderr) on the tracing subscriber. The shutdown integration tests already capture combined stdout+stderr (read_output), so they still observe the same lifecycle/readiness/degrade lines; only stale doc comments were corrected, no assertion changed. Harmless under systemd (both -> journal). Dual gate GO/GO. Commit 802e80e.
<!-- SECTION:FINAL_SUMMARY:END -->
