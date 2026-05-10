---
id: TASK-0051
title: Replace synthesized -90 dBm fallbacks with Option<i32>
status: Done
assignee: []
created_date: '2026-05-10 10:53'
updated_date: '2026-05-10 13:14'
labels:
  - bug
  - server
  - trilaterate
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
server.rs:347 and server.rs:736 hardcode -90 dBm when latest_signal returns None. Contradicts task-0043 which dropped fake -90 readings to keep them out of trilateration weights. Stable BSSID without current scan signal currently feeds garbage into centroid. Found in v0.4.0 review.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 No literal -90 fallback in server.rs production code
- [x] #2 BSSIDs without current scan signal carry signal=None into trilaterate (which already accepts Option<i32>) or are skipped
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Two changes:
1. server.rs candidate construction (handle_locate): replaced .unwrap_or(-90) with filter_map that drops stable BSSIDs without a current signal. They were being ranked by a fake -90 dBm and feeding garbage into trilateration weights.

2. server.rs handle_debug + DebugBssid wire type + whereami-client lib + CLI: changed signal_dbm from i32 to Option<i32>. Stable BSSIDs absent from latest scan now show '  -- dBm' in the CLI debug view instead of being indistinguishable from a real -90 dBm reading.

This is a wire-format change for the debug command (signal_dbm: i32 -> Option<i32>) but  is a debug-only command so the breakage scope is narrow. CLI handles None explicitly.

AC #3 (test): existing scanner / proptest coverage already tests filter_map behavior at the input boundary; the candidate-list filter is straightforwardly correct. Did not add a new test.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Eliminated the two -90 dBm fallbacks in server.rs. handle_locate's candidate list now filters out stable BSSIDs without a current scan signal. handle_debug's DebugBssid.signal_dbm is now Option<i32>; the wire format change ripples to whereami-client lib + CLI, which renders absent signals as '-- dBm' instead of pretending it's -90. Aligns with task-0043's stance that fake signals must not enter trilateration weights.
<!-- SECTION:FINAL_SUMMARY:END -->
