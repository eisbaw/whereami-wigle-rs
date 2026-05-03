---
id: TASK-4
title: Wi-Fi scanner via iw scan trigger/dump
status: Done
assignee: []
created_date: '2026-05-02 00:05'
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
Implement scanner.rs: shell out to iw dev <iface> scan trigger then iw dev <iface> scan dump. Parse BSS entries from iw output: extract BSSID, SSID, signal strength (dBm), channel, frequency. Return Vec of scan results sorted by signal strength. Handle 'Device or resource busy' gracefully (retry after short delay). Records ALL observed APs (not just top-N).
<!-- SECTION:DESCRIPTION:END -->
