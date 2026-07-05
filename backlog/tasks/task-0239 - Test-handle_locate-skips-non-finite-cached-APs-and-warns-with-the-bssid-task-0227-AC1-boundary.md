---
id: TASK-0239
title: >-
  Test: handle_locate skips non-finite cached APs and warns with the bssid
  (task-0227 AC#1 boundary)
status: Done
assignee:
  - task0239-implementer
created_date: '2026-07-04 17:18'
updated_date: '2026-07-05 12:48'
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
- [x] #1 a handle_locate-level test seeds the aps cache with a non-finite-coord AP among finite ones and asserts the non-finite AP does not contribute to the returned position
- [x] #2 the skip is observable: assert the warn! names the bssid via a tracing capture, OR document why only the behavioral skip is asserted
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Reuse the real handle_locate dispatch fixture introduced by TASK-0093, but isolate one finite cluster plus a named non-finite cached AP so the server boundary—not only trilaterate—must skip it.
2. Add a deterministic tracing subscriber/capture local to the server test and assert exactly that a WARN event names the poisoned BSSID and non-finite cached position while the returned fix remains the finite cluster.
3. Run the focused test repeatedly, formatting/lint/full e2e, freeze the diff, pass parallel QA and architecture review, close the tracker, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Selected by the Phase 3 orchestrator as the only explicit dependency sequence item. TASK-0093 already added handle_locate stable/fallback regressions with non-finite rows, satisfying the behavioral direction indirectly; this task remains open because no test captures the required WARN event naming the poisoned BSSID.

Completion: Added a real Request::Locate boundary regression with an in-memory database and stable scanner state. The strongest poisoned AP carries +Inf latitude; assertions prove it remains visible/stable but is excluded from cached/source accounting and the returned finite location. A future-local tracing subscriber captures and pins the WARN level, server target, poisoned BSSID, and rendered non-finite coordinates without global subscriber races. Implementer validation: focused test 10/10, dispatch suite 33/33, mutation removing the admission return failed, just fmt/lint passed, and just e2e passed 558 tests. Mandatory QA review: GO, focused 20/20, dispatch 33/33, just e2e 558, strict Clippy and diff checks pass. Mandatory architecture review: GO, daemon suite 340/340, strict Clippy and fmt pass, independent admission and warning mutations both failed as expected. Non-blocking scope note: this boundary fixture uses +Inf latitude; broader non-finite variants remain covered below this boundary and production checks both coordinates symmetrically.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added and validated a real handle_locate regression proving non-finite cached APs are excluded and emit a precise BSSID-bearing warning. Both mandatory review gates passed with independent mutation checks and the full E2E suite.
<!-- SECTION:FINAL_SUMMARY:END -->
