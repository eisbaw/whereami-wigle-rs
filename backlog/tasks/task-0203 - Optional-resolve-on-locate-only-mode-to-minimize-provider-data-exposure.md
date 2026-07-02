---
id: TASK-0203
title: Optional resolve-on-locate-only mode to minimize provider data exposure
status: To Do
assignee: []
created_date: '2026-07-02 04:52'
labels:
  - privacy
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The scan loop proactively sends every newly-stable BSSID to Apple WPS (then WiGLE) in the background, continuously, whether or not the user ever issues a locate. There is no mode to defer resolution until an actual position query, so the provider receives a continuous stream of the user's Wi-Fi environment even when idle. Ref whereamid/src/main.rs:246-253, resolver.rs:437-452.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 a config flag exists where no BSSID is sent to any provider until the user requests a position (accepting higher first-fix latency)
- [ ] #2 the default behavior and its privacy/latency tradeoff are documented
<!-- AC:END -->
