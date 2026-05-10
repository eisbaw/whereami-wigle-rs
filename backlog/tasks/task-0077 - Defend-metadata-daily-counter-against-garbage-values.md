---
id: TASK-0077
title: Defend metadata daily-counter against garbage values
status: Done
assignee: []
created_date: '2026-05-10 11:01'
updated_date: '2026-05-10 13:22'
labels:
  - db
  - robustness
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
db.rs api_calls_today does String::parse().unwrap_or(0). If anyone pokes the metadata table by hand and writes garbage, the counter silently resets to 0 and the daemon happily re-charges its WiGLE quota. Add CHECK constraint on metadata rows for the rate-limit keys, OR a typed accessor that errors loudly. Found in v0.4.0 review (swe-gardener).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Garbage value in metadata.api_calls_today produces a typed error or a loud warn (not silent reset to 0)
- [x] #2 Test: insert 'not-a-number' into metadata, call api_calls_today; assert behavior
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added parse_api_calls_value(Option<String>) -> u32 free function that warns on parse failure instead of silently returning 0. Used in three sites: api_calls_today, try_reserve_api_call, refund_api_call. Garbage in the metadata table still falls back to 0 (counter resets, daemon recovers) but the corruption is now visible in logs. Did not switch to a CHECK constraint because metadata is a typeless KV table; a CHECK on a single key is awkward and a typed accessor with logging is cheaper.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Garbage in metadata.api_calls_today now logs a warn instead of silently resetting the counter. Three call sites use a shared parse helper; behavior on garbage is still safe (treat as 0) but no longer invisible.
<!-- SECTION:FINAL_SUMMARY:END -->
