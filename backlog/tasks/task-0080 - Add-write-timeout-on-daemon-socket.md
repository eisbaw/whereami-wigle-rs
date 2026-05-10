---
id: TASK-0080
title: Add write timeout on daemon socket
status: Done
assignee: []
created_date: '2026-05-10 11:02'
updated_date: '2026-05-10 13:22'
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
- [x] #1 Writes to client TCP stream have a timeout (e.g. 5s) similar in magnitude to READ_TIMEOUT
- [x] #2 Slow-read client cannot pin a server task indefinitely
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added WRITE_TIMEOUT = 5s and wrapped writer.write_all + write_all + shutdown in tokio::time::timeout in handle_connection. On timeout the connection is dropped and the spawned task exits. Symmetric to the existing READ_TIMEOUT.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Daemon socket now has a 5s write timeout symmetric to READ_TIMEOUT. A slow-reading client cannot pin a tokio task with a buffered response anymore.
<!-- SECTION:FINAL_SUMMARY:END -->
