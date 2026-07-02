---
id: TASK-0199
title: whereami | head panics with exit 101 (no broken-pipe handling)
status: To Do
assignee: []
created_date: '2026-07-02 04:52'
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
- [ ] #1 whereami scan | head -1 and whereami --json debug | head -c 10 exit without a panic message
- [ ] #2 exit status under early-closed pipes is 0 (or a documented code), never 101 (reset SIGPIPE to SIG_DFL or map BrokenPipe to clean exit)
- [ ] #3 behavior covered by a test or documented in CLI help
<!-- AC:END -->
