---
id: TASK-0026
title: Add source priority to upsert_ap
status: To Do
assignee: []
created_date: '2026-05-09 21:07'
labels: []
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
When a BSSID is resolved by both Apple and WiGLE, whoever writes last wins. Apple positions are generally more accurate. upsert_ap should prefer Apple over WiGLE: only overwrite if new source has equal or higher priority (apple > wigle > beacondb > manual).
<!-- SECTION:DESCRIPTION:END -->
