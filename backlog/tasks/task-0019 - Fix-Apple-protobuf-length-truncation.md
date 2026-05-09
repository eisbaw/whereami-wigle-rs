---
id: TASK-0019
title: Fix Apple protobuf length truncation
status: To Do
assignee: []
created_date: '2026-05-09 21:07'
labels: []
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
apple.rs line 100: proto.len() as u8 silently truncates for payloads >255 bytes. Batch of 15+ BSSIDs produces garbage. Either encode as multi-byte length or validate/cap input size and fail loudly.
<!-- SECTION:DESCRIPTION:END -->
