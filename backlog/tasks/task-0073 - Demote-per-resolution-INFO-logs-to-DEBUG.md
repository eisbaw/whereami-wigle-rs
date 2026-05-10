---
id: TASK-0073
title: Demote per-resolution INFO logs to DEBUG
status: To Do
assignee: []
created_date: '2026-05-10 11:00'
labels:
  - observability
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
resolver.rs:238-244 logs every successful BSSID resolution at INFO including BSSID, lat, lon. With Apple WPS being free and unbounded, a long fast-scan phase in a busy area fills the journal with these. Move to debug! and keep INFO for higher-level state transitions. Found in v0.4.0 review (keeper).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Per-BSSID 'resolved' log line is debug! not info!
- [ ] #2 Higher-level INFO logs preserved (scan summary, fast/slow phase transitions, drain summary)
<!-- AC:END -->
