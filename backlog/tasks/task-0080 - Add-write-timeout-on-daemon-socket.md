---
id: TASK-0080
title: Add write timeout on daemon socket
status: To Do
assignee: []
created_date: '2026-05-10 11:02'
labels:
  - server
  - robustness
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
server.rs:18 has READ_TIMEOUT=5s but no write timeout. A slow client that connects, sends a request, then reads the response slowly will pin a tokio task with the response buffered. Wrap writer.write_all + writer.shutdown in tokio::time::timeout. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Writes to client TCP stream have a timeout (e.g. 5s) similar in magnitude to READ_TIMEOUT
- [ ] #2 Slow-read client cannot pin a server task indefinitely
<!-- AC:END -->
