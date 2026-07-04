---
id: TASK-0240
title: 'whereamid logs to stdout (tracing default), not stderr - decide/document'
status: To Do
assignee: []
created_date: '2026-07-04 17:33'
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
- [ ] #1 A decision is recorded: stderr for convention, OR documented stdout-by-design with rationale
- [ ] #2 If changed to stderr: the subscriber uses .with_writer(std::io::stderr) and tests/consumers are updated; if kept on stdout: a comment at the subscriber init documents it
<!-- AC:END -->
