---
id: TASK-0057
title: Convert Request to serde-tagged enum
status: Done
assignee: []
created_date: '2026-05-10 10:55'
updated_date: '2026-05-10 13:40'
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
- [x] #1 Request is a tagged enum; each variant carries only its own fields
- [x] #2 Old wire format still parses (existing CLI continues to work)
- [x] #3 Unknown commands and missing required fields produce typed errors at deserialization
- [x] #4 dispatch_command is an exhaustive match over the enum
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Request is now #[serde(tag='cmd', rename_all='lowercase')] enum with variants Locate/Resolve{bssids}/Scan/Stats/Debug/Version/History{range,from,to}. dispatch_command is an exhaustive match. Old wire format ({"cmd":"locate"}, {"cmd":"resolve","bssids":[]}, etc.) parses identically. Unknown commands now produce a serde error rather than reaching dispatch_command. Refactored handle_history to take (range, from, to) directly instead of the whole Request.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Request struct -> serde-tagged enum. dispatch_command is exhaustive. Wire format unchanged. Removed the implicit illegal states (e.g. cmd=locate with bssids set).
<!-- SECTION:FINAL_SUMMARY:END -->
