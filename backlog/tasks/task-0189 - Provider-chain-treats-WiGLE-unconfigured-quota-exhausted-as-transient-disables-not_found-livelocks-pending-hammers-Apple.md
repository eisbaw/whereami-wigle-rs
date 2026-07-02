---
id: TASK-0189
title: >-
  Provider chain treats WiGLE-unconfigured/quota-exhausted as transient:
  disables not_found, livelocks pending, hammers Apple
status: To Do
assignee: []
created_date: '2026-07-02 04:50'
labels:
  - correctness
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
wigle_lookup returns Skipped both when creds are absent and when the daily quota is gone. In resolve_chain, Skipped -> transient_error -> the chain-end guard skips the not_found mark for any BSSID with a transient outcome. So in the advertised credential-free mode NO BSSID is ever marked not_found: every scan re-runs an Apple WPS call per unknown AP forever; drain attempts only increment on NetworkError so Apple-NotFound+WiGLE-Skipped rows sit at attempts=0 and never age out; get_pending ORDER BY attempts ASC makes >=10 immortal rows monopolise every drain and starve newer entries. Same daily once a configured user's 100-call quota is exhausted. Ref whereamid/src/provider.rs:118-139, resolver.rs:297-305,376-390, pending.rs:110-125, db.rs:581-594,631-637.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 with WiGLE unconfigured, a BSSID Apple definitively reports NotFound is marked not_found and not re-queried on subsequent scans
- [ ] #2 pending rows whose every drain pass ends without success eventually age out via pending_max_attempts
- [ ] #3 with >10 unresolvable pending rows, other pending BSSIDs still get drained (no head-of-line monopoly)
- [ ] #4 quota-exhausted days do not disable not_found marking for Apple-definitive misses
<!-- AC:END -->
