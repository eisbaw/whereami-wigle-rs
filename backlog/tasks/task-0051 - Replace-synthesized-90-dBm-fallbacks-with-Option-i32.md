---
id: TASK-0051
title: Replace synthesized -90 dBm fallbacks with Option<i32>
status: To Do
assignee: []
created_date: '2026-05-10 10:53'
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
- [ ] #1 No literal -90 fallback in server.rs production code
- [ ] #2 BSSIDs without current scan signal carry signal=None into trilaterate (which already accepts Option<i32>) or are skipped
<!-- AC:END -->
