---
id: TASK-0246
title: Reject stale scan snapshots in autonomous locate
status: To Do
assignee: []
created_date: '2026-07-05 14:24'
labels:
  - correctness
  - history
dependencies:
  - TASK-0096
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0096 intentionally consumes the existing Debouncer rather than owning a second scanner. The Debouncer retains its last stable snapshot indefinitely across scanner failures, so an autonomous loop can otherwise append stationary fixes long after the last real observation. Define and enforce an explicit freshness policy without changing interactive locate semantics.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Define a documented monotonic maximum scan-age policy for autolocate, including its relationship to scan cadence and configurable intervals.
- [ ] #2 Autolocate skips before cache touch, plausibility, last_fix mutation, history append, and counter increment when the latest scan is absent or stale; interactive locate behavior remains unchanged.
- [ ] #3 Expose any new threshold through Rust CLI and shared NixOS/Home Manager options with zero/disable semantics pinned and flag/default parity tested.
- [ ] #4 Add deterministic fresh-boundary, stale-boundary, repeated scanner-failure, recovery-after-new-scan, wipe, and shutdown tests.
- [ ] #5 Document that freshness prevents autonomous history from treating an old Wi-Fi snapshot as new motion evidence.
<!-- AC:END -->
