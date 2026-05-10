---
id: TASK-0059
title: Add From<&LastFix> for LastFixRow + extract test_state to common module
status: To Do
assignee: []
created_date: '2026-05-10 10:55'
labels:
  - refactor
  - types
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Two open-coded marshalling blocks build a LastFixRow from a LastFix (server.rs:469-499 backfill, server.rs:533-541 handle_locate). main.rs:64-87 marshals back. A From/TryFrom pair plus a tests/common.rs holding the duplicate test_state() (resolver.rs:428, pending.rs:152) cleans this up. Found in v0.4.0 review (swe-gardener, mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 From<&LastFix> for LastFixRow and TryFrom<&LastFixRow> for LastFix impls in db.rs
- [ ] #2 Marshalling at all three call sites uses the conversions
- [ ] #3 test_state() lives in one place (test helpers crate or tests/common.rs)
<!-- AC:END -->
