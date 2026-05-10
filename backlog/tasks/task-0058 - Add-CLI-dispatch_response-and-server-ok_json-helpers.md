---
id: TASK-0058
title: Add CLI dispatch_response and server ok_json helpers
status: Done
assignee: []
created_date: '2026-05-10 10:55'
updated_date: '2026-05-10 13:39'
labels:
  - refactor
  - cleanup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
whereami-client/src/main.rs has 6 near-identical match ladders (Ok if json => raw JSON; Ok if !ok => fatal; Ok => render; Err => fatal). server.rs has 6+ identical 'serde_json::to_string(&resp).unwrap_or_else(|_| error_json(...))' tails. Two small helpers eliminate ~80 LOC of boilerplate. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 CLI dispatch_response<T>(result, json, render: impl Fn(&T)) used by every subcommand
- [x] #2 Server ok_json<T: Serialize>(t: &T) -> String used by every handler
- [x] #3 Net LOC reduction visible in diff
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added ok_json<T: Serialize>(t: &T) -> String in server.rs replacing 8 identical 'serde_json::to_string(&resp).unwrap_or_else(|_| error_json(...))' tails. Added DaemonResponse trait in whereami-client/lib.rs with impl_daemon_response! macro registering all 7 response types. Added dispatch<T: Serialize + DaemonResponse>(result, json, render) in whereami-client/main.rs replacing 5 near-identical match ladders (kept 'version' as-is because it has unique cli-version-on-error rendering).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Helpers added; ~80 LOC of boilerplate eliminated. ok_json in server, dispatch + DaemonResponse trait in CLI.
<!-- SECTION:FINAL_SUMMARY:END -->
