---
id: TASK-0057
title: Convert Request to serde-tagged enum
status: To Do
assignee: []
created_date: '2026-05-10 10:55'
labels:
  - refactor
  - server
  - wire-format
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
server.rs:157-172 Request is { cmd: String, bssids, range, from, to } with optional fields polluting every command. Replace with #[serde(tag='cmd')] enum Request { Locate, Resolve { bssids }, History { range, from, to }, ... }. dispatch_command becomes an exhaustive match. Backwards-compatible wire format. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Request is a tagged enum; each variant carries only its own fields
- [ ] #2 Old wire format still parses (existing CLI continues to work)
- [ ] #3 Unknown commands and missing required fields produce typed errors at deserialization
- [ ] #4 dispatch_command is an exhaustive match over the enum
<!-- AC:END -->
