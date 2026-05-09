---
id: TASK-0025
title: Nominatim should not block locate response
status: To Do
assignee: []
created_date: '2026-05-09 21:07'
labels: []
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
handle_locate calls Nominatim synchronously in the response path. Slow Nominatim (1-5s) blocks the locate response even though position was computed instantly. Nominatim rate limiter mutex also serializes concurrent locate requests. Fix: return position immediately, resolve address asynchronously and cache it.
<!-- SECTION:DESCRIPTION:END -->
