---
id: TASK-0264
title: 'Reconsider lookup_cached &[Candidate] signature + locate loop single-probe'
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 21:52'
updated_date: '2026-07-08 22:02'
labels:
  - refactor
  - server
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0140 (commit 01f748a). Two deferred items:
1. AC #2 of task-0140 suggested lookup_cached take &[Candidate] and return positioned rows directly. Deferred: it would ripple into resolver.rs autolocate caller (~line 1228) and the lookup_cached_readonly sibling, both &[String], for no readability gain. Revisit only if a future change already touches those callers.
2. To hold handle_locate net line delta at 0, the positioned_aps loop reads the signal via active.get(bssid).map(...) inside a branch already gated by active.contains_key(bssid) — a second (cheap) probe of a small in-memory map. A single-lookup `if let Some(candidate) = active.get(...)` bind is marginally cleaner but costs +1 line. Decide whether the +1 line is acceptable for the single-lookup form.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Decide lookup_cached signature (keep &[String] or move to &[Candidate]) with rationale recorded
- [x] #2 Decide locate loop form (double-probe vs single if-let bind) and apply chosen form
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Resolved both deferred decisions from task-0140.

AC#1 (lookup_cached signature): DECISION = keep &[String] (do NOT move to &[Candidate]). Rationale: it would ripple into lookup_cached_readonly and the resolver.rs:1228 autolocate caller, both &[String], for zero readability gain — the cache layer legitimately operates in bssid-string terms. mped-architect endorsed this in the 0140 review. No code change.

AC#2 (locate loop form): DECISION = single if-let bind, APPLIED. The positioned_aps loop now does `if let Some(candidate) = active.get(bssid) { if len < top_n { push(signal_dbm: Some(candidate.signal_dbm)) } }` instead of `contains_key(..) && len<top_n` + a second `active.get(..).map(..)` probe. One hash lookup; states the always-Some invariant honestly. The +1 line (nested if, since edition 2021 lacks let-chains) is accepted for the clarity + single-lookup win.

Verified (orchestrator, behavior-preserving): contains_key ⟺ get.is_some on the same map and the signal was always Some, so identical behavior — dispatch_tests 71/71 unchanged, clippy -D warnings + fmt clean. Commit above.
<!-- SECTION:FINAL_SUMMARY:END -->
