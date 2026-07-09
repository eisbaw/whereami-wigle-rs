---
id: TASK-0274
title: Cap the scanner already-warned malformed-BSSID set (unbounded in principle)
status: To Do
assignee: []
created_date: '2026-07-09 05:48'
labels:
  - scanner
  - observability
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0256 review LOW: scanner.rs malformed_signal_warn_is_first uses a never-cleared process-global OnceLock<Mutex<HashSet<String>>> keyed iw:/nmcli:<bssid>. Bounded in practice (one small entry per distinct malformed-signal BSSID the LOCAL driver ever emits; the malformed field is local-driver-sourced, not attacker-drivable over the air), but unbounded in principle over a very long daemon lifetime in an RF environment cycling many distinct malformed BSSIDs. Optional hardening: cap the set (once size > N, always DEBUG) or LRU-evict. LOW — the adversarial path is constrained; current behavior is acceptable + documented.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 The already-warned set cannot grow without bound: either a size cap (beyond N, malformed-signal repeats always DEBUG) or an eviction policy, with a test
<!-- AC:END -->
