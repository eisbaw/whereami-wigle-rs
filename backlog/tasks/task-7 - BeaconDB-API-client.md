---
id: TASK-7
title: BeaconDB API client
status: Done
assignee: []
created_date: '2026-05-02 00:06'
updated_date: '2026-05-02 00:13'
labels: []
dependencies:
  - TASK-1
references:
  - PRD.md
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement beacondb.rs: HTTP POST to https://beacondb.net/v1/geolocate with wifiAccessPoints array. No auth. Returns lat/lon/accuracy. Detect IP fallback (fallback field in response) and treat as not-found. Accept multiple BSSIDs in one call (batch). configurable enable/disable via config.
<!-- SECTION:DESCRIPTION:END -->
