---
id: TASK-0131
title: Rename fix vocabulary to wifi_fix / position_estimate
status: To Do
assignee: []
created_date: '2026-05-20 21:20'
labels:
  - docs
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #10. 'Fix' in geolocation almost always means GPS fix (satellite-derived position). This daemon never sees GPS — every 'fix' is a Wi-Fi centroid estimate. db.rs:75 FixRow, db.rs:88 LastFixRow, server.rs LastFix, schema 'fixes' table. A reader from GPS background will assume accuracy_m has GPS semantics (HDOP-derived radius) — see task-0127 for why it doesn't.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Doc-comments on FixRow, LastFixRow, LastFix explicitly say: 'a Wi-Fi-trilateration-derived position estimate, NOT a GPS fix'
- [ ] #2 Optional (heavier): rename to WifiFixRow, LastWifiFix, schema table 'wifi_fixes' via migration v6 (CLI flags --wifi-history-* etc.) — this is the cheapest of the Compass findings to JUST DOC but the heaviest to fully rename
- [ ] #3 Decision recorded in task notes: doc-only OR full rename
<!-- AC:END -->
