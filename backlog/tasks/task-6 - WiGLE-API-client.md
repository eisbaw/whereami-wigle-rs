---
id: TASK-6
title: WiGLE API client
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
Implement wigle.rs: HTTP client using reqwest. Single method lookup_bssid(bssid) -> Result<Option<ApInfo>>. Uses HTTP Basic auth from config. Returns lat, lon, ssid, encryption, channel, frequency, city, country from the trilat/trilong fields. Handle 404 (return None), 429 (return rate-limit error), network errors. No retry logic here - caller handles that.
<!-- SECTION:DESCRIPTION:END -->
