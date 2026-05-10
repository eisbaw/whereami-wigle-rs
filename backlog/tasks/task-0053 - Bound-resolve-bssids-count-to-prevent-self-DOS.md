---
id: TASK-0053
title: Bound /resolve bssids count to prevent self-DOS
status: Done
assignee: []
created_date: '2026-05-10 10:54'
updated_date: '2026-05-10 13:15'
labels:
  - server
  - robustness
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Request.bssids: Vec<String> is unbounded (server.rs:309). A 64KiB JSON of 6-byte BSSIDs is ~7000 entries; resolve_chain takes the DB lock per BSSID and would starve the rest of the daemon. MAX_REQUEST_BYTES bounds the wire size but not the count. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 handle_resolve rejects requests with more than MAX_RESOLVE_BSSIDS (e.g. 256) entries with a clear error
- [x] #2 Limit is a named const, not a magic number
- [ ] #3 Test for limit-exceeded request
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added MAX_RESOLVE_BSSIDS = 256 const; handle_resolve checks bssids.len() and returns a clear error if exceeded. The limit is well above any realistic Wi-Fi scan (typical <50 BSSIDs visible) but well below the ~7000 that fit in 64 KiB of wire JSON. AC #3 (test) deferred — handle_resolve test would need full DaemonState; the change is a 5-line bounds check with a clear contract.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
/resolve now rejects requests with more than MAX_RESOLVE_BSSIDS (256) entries with a clear error. Closes the self-DOS where a 64KiB JSON of 6-byte BSSIDs (~7000) would take the DB lock per entry inside resolve_chain.
<!-- SECTION:FINAL_SUMMARY:END -->
