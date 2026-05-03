---
id: TASK-15
title: whereami-client library crate
status: Done
assignee: []
created_date: '2026-05-02 00:06'
updated_date: '2026-05-02 00:20'
labels: []
dependencies:
  - TASK-12
references:
  - PRD.md
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement whereami-client/src/lib.rs: connect to TCP socket, send JSON command, read JSON response, parse into typed structs. Methods: locate(), resolve(bssids), scan(), stats(). Each opens a new TCP connection (one-shot). Serde types for all request/response shapes. Configurable address (default 127.0.0.1:4747).
<!-- SECTION:DESCRIPTION:END -->
