---
id: TASK-0038
title: CLI-configurable HTTP timeouts
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 05:38'
updated_date: '2026-05-10 08:19'
labels:
  - config
  - enhancement
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0020 added explicit HTTP timeouts (5s connect, 15s total fast / 30s Nominatim) but they are compile-time constants in whereamid/src/http.rs. Some users in lossy networks may need longer timeouts; add CLI/config options to override. Single --http-timeout-secs that applies to fast endpoints, plus --nominatim-timeout-secs is enough.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Args has --http-timeout-secs and --nominatim-timeout-secs (or equivalent config keys)
- [x] #2 PRD updated to reflect that timeouts are now configurable
- [x] #3 Defaults match the current compile-time constants (15s, 30s)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add CLI flags --http-timeout-secs (default 15) and --nominatim-timeout-secs (default 30)
2. Pass timeouts to client constructors instead of using compile-time constants
3. Update PRD to note timeouts are now configurable
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added --http-timeout-secs (default 15) and --nominatim-timeout-secs (default 30). Validated > 0 in Args::validate(). Wired through to AppleClient::with_timeout, WigleClient::with_timeout, NominatimClient::with_timeout in main.rs. The compile-time constants REQUEST_TIMEOUT_FAST and REQUEST_TIMEOUT_NOMINATIM are now used only as test/default-constructor fallbacks. PRD updated to note timeouts are configurable.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
HTTP timeouts are now CLI-configurable. --http-timeout-secs (default 15) controls Apple WPS and WiGLE total request timeout. --nominatim-timeout-secs (default 30) controls Nominatim. Connect timeout stays at compile-time 5s. Validated > 0 at parse time. PRD updated.
<!-- SECTION:FINAL_SUMMARY:END -->
