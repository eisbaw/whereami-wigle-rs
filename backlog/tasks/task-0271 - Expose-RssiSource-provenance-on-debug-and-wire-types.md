---
id: TASK-0271
title: Expose RssiSource provenance on debug and wire types
status: To Do
assignee: []
created_date: '2026-07-09 02:56'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up to task-0126. task-0126 introduced Rssi/RssiSource and threads provenance (iw authoritative dBm vs nmcli biased linearization) in-memory from scanner to PositionedAp, but the wire/debug types NetworkEntry.signal_dbm and DebugBssid.signal_dbm still expose only a bare i32. An operator inspecting scan/debug output cannot see whether an AP dBm is authoritative or a linearized approximation. Consider adding an optional source field to the debug wire so provenance is visible externally. Requires a whereami-protocol change, deliberately out of scope for task-0126 which was daemon-internal only.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Debug/scan wire output surfaces per-AP RSSI provenance (iw vs nmcli-linearized)
- [ ] #2 whereami-protocol version and compat handled
<!-- AC:END -->
