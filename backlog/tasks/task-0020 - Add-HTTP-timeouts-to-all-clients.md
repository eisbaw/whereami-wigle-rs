---
id: TASK-0020
title: Add HTTP timeouts to all clients
status: To Do
assignee: []
created_date: '2026-05-09 21:07'
labels: []
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
reqwest::Client::new() has no timeout. If Apple/WiGLE/Nominatim hangs, background tasks block forever and leak memory. Build all clients with .timeout(Duration::from_secs(10)).
<!-- SECTION:DESCRIPTION:END -->
