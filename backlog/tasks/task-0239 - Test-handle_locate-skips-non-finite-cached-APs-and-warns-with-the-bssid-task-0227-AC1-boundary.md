---
id: TASK-0239
title: >-
  Test: handle_locate skips non-finite cached APs and warns with the bssid
  (task-0227 AC#1 boundary)
status: To Do
assignee: []
created_date: '2026-07-04 17:18'
labels:
  - testing
  - daemon
dependencies:
  - TASK-0227
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0227 AC#1 (server-side skip of non-finite cached APs with a warn! naming the bssid, server.rs ~838-855) is currently only covered indirectly by the trilaterate-level tests; the handle_locate boundary itself has no test. Constructible today: test_state() (server.rs ~1392), Debouncer::push_scan (injectable, already used in tests), and upsert_ap accept arbitrary f64 (incl. NaN). Such a test would also cover the currently-untested last_fix backfill keys_match logic. Asserting the warn! TEXT may need a tracing-capture layer (tracing-test or a custom subscriber) - evaluate asserting the log vs only the behavioral skip. Line numbers approximate.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 a handle_locate-level test seeds the aps cache with a non-finite-coord AP among finite ones and asserts the non-finite AP does not contribute to the returned position
- [ ] #2 the skip is observable: assert the warn! names the bssid via a tracing capture, OR document why only the behavioral skip is asserted
<!-- AC:END -->
