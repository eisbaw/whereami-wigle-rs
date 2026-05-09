---
id: TASK-0028
title: CLI should use typed response structs
status: To Do
assignee: []
created_date: '2026-05-09 21:07'
labels: []
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
whereami-client main.rs uses raw_command + serde_json::Value for locate and scan, bypassing the typed LocateResponse/ScanResponse structs in lib.rs. If the daemon changes a field name, the compiler wont catch it. Add stale/age_s/scanned_at fields to the typed structs and use them.
<!-- SECTION:DESCRIPTION:END -->
