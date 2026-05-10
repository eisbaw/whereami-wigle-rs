---
id: TASK-0060
title: Trim PendingAp struct or use its metadata
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-10 10:56'
updated_date: '2026-05-10 16:28'
labels:
  - cleanup
  - db
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
db.rs:94 PendingAp has bssid, ssid, channel, frequency, signal_dbm, attempts. pending.rs:89 only reads bssid and attempts; the rest are loaded but never consumed. Either trim get_pending() to just (bssid, attempts) and drop the struct, or actually use the metadata to inform retry policy (signal-strength-weighted retry?). Found in v0.4.0 review (mped-architect, keeper).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 PendingAp fields all have at least one consumer, OR struct is removed and get_pending returns Vec<(String, i32)>
- [x] #2 No #[allow(dead_code)] on PendingAp
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Trim PendingAp to {bssid, attempts}; narrow get_pending SQL accordingly.
2. Drop the #[allow(dead_code)] annotation.
3. Update the existing tests in db.rs which already only consume bssid+attempts.
4. cargo build + test + clippy + fmt-check inside nix develop.
5. Commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Deferred during Phase 5: trimming PendingAp's unread fields (ssid/channel/frequency/signal_dbm) requires touching get_pending SQL + the PendingAp struct + any consumers. Risk-vs-reward unfavorable for current sweep — the unread fields cost <100 bytes per row and the fields themselves serve as documentation of what data we receive on insert. Re-open when PendingAp gains a new consumer, or when get_pending becomes a hot path.

Removed PendingAp struct. get_pending now returns Vec<(String, i32)>; updated the one production consumer (pending.rs) to destructure the tuple, and the two existing db.rs tests to index .0/.1.

cargo build / test (9 passed) / clippy (clean) / fmt --check (clean) all green inside nix develop.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Removed PendingAp.

The four metadata fields (ssid/channel/frequency/signal_dbm) had no production consumer and existed only to suppress a dead_code lint. get_pending now returns Vec<(String, i32)>; the one production caller (pending.rs::drain_once) destructures the tuple. Tests rewritten to index by tuple position. The pending SQL table still stores the metadata for diagnostics — re-add a struct (and widen the SELECT) the moment a real consumer appears.

No behavior change. cargo build / test / clippy / fmt all clean.
<!-- SECTION:FINAL_SUMMARY:END -->
