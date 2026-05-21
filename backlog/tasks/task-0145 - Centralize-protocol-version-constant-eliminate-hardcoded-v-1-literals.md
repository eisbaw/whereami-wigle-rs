---
id: TASK-0145
title: 'Centralize protocol version constant (eliminate hardcoded v: 1 literals)'
status: To Do
assignee: []
created_date: '2026-05-20 21:27'
labels:
  - hygiene
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Hardcoded 'v': 1 protocol version literal scattered across every response struct — ~8 sites in server.rs, every wire struct in client/lib.rs. When v=2 happens, sed-edit in 16+ places. A const WIRE_VERSION: u32 = 1; referenced everywhere is one-line free insurance.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Define const WIRE_VERSION: u32 = 1; in whereami-protocol crate (or shared module if 0114 hasn't landed yet)
- [ ] #2 All response struct construction sites use WIRE_VERSION instead of 1
- [ ] #3 Doc-comment on WIRE_VERSION: 'breaking wire-format changes bump this; clients can use it to detect daemon protocol version'
<!-- AC:END -->
