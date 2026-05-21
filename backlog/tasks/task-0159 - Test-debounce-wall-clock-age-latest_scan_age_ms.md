---
id: TASK-0159
title: 'Test: debounce wall-clock age (latest_scan_age_ms)'
status: To Do
assignee: []
created_date: '2026-05-20 21:31'
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
- [ ] #1 Test: push a scan, tokio::time::sleep(50ms), assert latest_scan_age_ms() >= 50 && < 5000
- [ ] #2 Test: push a scan, sleep, assert latest_scan_time matches the wall-clock at push time within drift tolerance
<!-- AC:END -->
