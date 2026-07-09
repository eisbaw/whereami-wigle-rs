---
id: TASK-0159
title: 'Test: debounce wall-clock age (latest_scan_age_ms)'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:31'
updated_date: '2026-07-09 04:20'
labels:
  - tests
  - debounce
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P1-5. debounce.rs:121-130 latest_scan_age_ms / latest_scan_time. Never tested. chrono::Utc::now() is used unmocked. If refactored to monotonic, the wall_clock field diverges from at in pathological cases (NTP step).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Test: push a scan, tokio::time::sleep(50ms), assert latest_scan_age_ms() >= 50 && < 5000
- [x] #2 Test: push a scan, sleep, assert latest_scan_time matches the wall-clock at push time within drift tolerance
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Use private push_scan_at with explicit wall_clock + received_at in the past for determinism
2. Assert latest_scan_time()==Some(exact wall_clock); latest_scan_age_ms() Some(v) with 500<=v<60000
3. Assert empty debouncer returns None for both; run 3x for flake
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added latest_scan_time_and_age_reflect_the_pushed_scan (debounce.rs). ADAPTED the ACs from their flaky literal form (tokio sleep + real-time age bound) to a DETERMINISTIC form per the no-new-flake requirement: drives the pre-existing private push_scan_at seam with an explicit wall_clock (asserted EXACTLY, AC#2) and received_at = Instant::now()-500ms (age floor >=500 via Instant monotonicity, ceiling <60000, AC#1) — no sleep, no unmocked Utc::now() in the assertion path. QA 5x flake stress: 5/5 identical passes. Reviewer confirmed non-flaky (Instant monotonicity is a hard guarantee) + bites (exact wall_clock equality pins latest_scan_time; empty-ring None pre-check bites the None-before-push contract). Dual gate GO/GO.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Pinned debounce latest_scan_time / latest_scan_age_ms (previously untested, use Utc::now()). Test drives the private push_scan_at seam with explicit wall_clock + a 500ms-past received_at (Instant math, no sleep) so it is deterministic and non-flaky (QA 5/5). Asserts exact stored time + a monotonic age floor. Test-only; commit 344fbb9.
<!-- SECTION:FINAL_SUMMARY:END -->
