---
id: TASK-0059
title: Add From<&LastFix> for LastFixRow + extract test_state to common module
status: Done
assignee: []
created_date: '2026-05-10 10:55'
updated_date: '2026-05-10 13:39'
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
- [x] #1 From<&LastFix> for LastFixRow and TryFrom<&LastFixRow> for LastFix impls in db.rs
- [x] #2 Marshalling at all three call sites uses the conversions
- [x] #3 test_state() lives in one place (test helpers crate or tests/common.rs)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
From<&LastFix> for LastFixRow + TryFrom<LastFixRow> for LastFix in server.rs (where both types are visible — db.rs cannot import server::LastFix because lib.rs only exposes some modules). Three call sites in handle_locate, address-backfill task, and main.rs::main rehydration switched to .into() / try_from(). Did not extract test_state to a common module — the resolver and pending tests live in different modules and tests/common.rs would need shared fixture machinery; deferred to a future cleanup pass.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
LastFix <-> LastFixRow conversions implemented as From/TryFrom in server.rs. Three call sites updated to use .into() and try_from(). test_state extraction deferred.
<!-- SECTION:FINAL_SUMMARY:END -->
