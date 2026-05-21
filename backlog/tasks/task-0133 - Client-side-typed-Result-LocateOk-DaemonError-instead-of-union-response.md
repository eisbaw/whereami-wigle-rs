---
id: TASK-0133
title: 'Client-side typed Result<LocateOk, DaemonError> instead of union response'
status: To Do
assignee: []
created_date: '2026-05-20 21:21'
labels:
  - refactor
  - wire-format
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/whereami-client/lib.rs:46-78. Client LocateResponse defines 'error: Option<String>' alongside the typed body. Daemon's LocateResponse has no 'error' field. Daemon serializes either LocateResponse{ok:true,...} OR ErrorResponse{ok:false,v,error}; client union-decodes via #[serde(default)]. Fragile coincidence: any future field collision (e.g. daemon adds 'error: ...' to LocateResponse for partial failure) silently mis-parses.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Client-side: enum DaemonResult<T> { Ok(T), Err { v, error } } with #[serde(untagged)] OR explicit dispatch on 'ok' bool field
- [ ] #2 Each public method (locate, scan, stats, debug, history, version, resolve) returns Result<TypedResp, DaemonError> where DaemonError carries the error string
- [ ] #3 The 'error' field is removed from each typed Response struct
- [ ] #4 Existing CLI behavior unchanged (CLI just unwraps to text in the same way)
- [ ] #5 Unit tests for each response shape: success path and error path
<!-- AC:END -->
