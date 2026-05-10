---
id: TASK-0060
title: Trim PendingAp struct or use its metadata
status: To Do
assignee: []
created_date: '2026-05-10 10:56'
updated_date: '2026-05-10 14:15'
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
- [ ] #1 PendingAp fields all have at least one consumer, OR struct is removed and get_pending returns Vec<(String, i32)>
- [ ] #2 No #[allow(dead_code)] on PendingAp
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Deferred during Phase 5: trimming PendingAp's unread fields (ssid/channel/frequency/signal_dbm) requires touching get_pending SQL + the PendingAp struct + any consumers. Risk-vs-reward unfavorable for current sweep — the unread fields cost <100 bytes per row and the fields themselves serve as documentation of what data we receive on insert. Re-open when PendingAp gains a new consumer, or when get_pending becomes a hot path.
<!-- SECTION:NOTES:END -->
