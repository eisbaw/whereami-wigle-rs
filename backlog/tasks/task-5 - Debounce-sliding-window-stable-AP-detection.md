---
id: TASK-5
title: 'Debounce: sliding window stable AP detection'
status: Done
assignee: []
created_date: '2026-05-02 00:06'
updated_date: '2026-05-02 00:20'
labels: []
dependencies:
  - TASK-4
references:
  - PRD.md
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement debounce.rs: ring buffer (VecDeque) of last N scan samples (configurable --debounce-window, default 10). Each sample is a HashMap<BSSID, signal_dbm>. Method is_stable(bssid) returns true if BSSID appears in >= M samples (configurable --debounce-threshold, default 5). Method stable_bssids() returns all currently stable BSSIDs. Method push_scan() adds a new scan sample and drops oldest if at capacity. All in-memory, no persistence.
<!-- SECTION:DESCRIPTION:END -->
