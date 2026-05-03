---
id: TASK-11
title: Weighted centroid trilateration
status: Done
assignee: []
created_date: '2026-05-02 00:06'
updated_date: '2026-05-02 00:13'
labels: []
dependencies:
  - TASK-1
references:
  - PRD.md
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement trilaterate.rs: given Vec<(lat, lon, signal_dbm)>, compute weighted centroid. Weight = 10^(signal_dbm / -20). If no signal info, equal weight. Return (lat, lon, accuracy_m). Accuracy estimated from weighted spread of input positions. Handle edge cases: single AP (return its position with low confidence), zero APs (return error).
<!-- SECTION:DESCRIPTION:END -->
