---
id: TASK-0022
title: Fix rate limit race condition
status: To Do
assignee: []
created_date: '2026-05-09 21:07'
labels: []
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Rate limit check (can_call_api) and record (record_api_call) are separated by an HTTP call. Multiple concurrent tasks can pass the check before any records the call, exceeding the daily limit. Use atomic counter or hold a separate Mutex/semaphore for API call gating.
<!-- SECTION:DESCRIPTION:END -->
