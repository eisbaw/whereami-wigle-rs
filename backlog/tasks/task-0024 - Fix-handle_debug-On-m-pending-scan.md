---
id: TASK-0024
title: Fix handle_debug O(n*m) pending scan
status: To Do
assignee: []
created_date: '2026-05-09 21:07'
labels: []
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
handle_debug calls db.get_pending(1000) inside a per-BSSID loop, scanning up to 1000 entries per AP. With 30 APs and 500 pending entries that is 15000 comparisons. Also holds debouncer lock during all DB queries. Fix: query pending once into a HashSet, drop debouncer lock before DB queries.
<!-- SECTION:DESCRIPTION:END -->
