---
id: TASK-0189
title: >-
  Provider chain treats WiGLE-unconfigured/quota-exhausted as transient:
  disables not_found, livelocks pending, hammers Apple
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-07-02 04:50'
updated_date: '2026-07-02 18:35'
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
- [x] #1 with WiGLE unconfigured, a BSSID Apple definitively reports NotFound is marked not_found and not re-queried on subsequent scans
- [x] #2 pending rows whose every drain pass ends without success eventually age out via pending_max_attempts
- [x] #3 with >10 unresolvable pending rows, other pending BSSIDs still get drained (no head-of-line monopoly)
- [x] #4 quota-exhausted days do not disable not_found marking for Apple-definitive misses
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. resolve_chain: replace single transient_error set with two: hard_transient (NetworkError+HardStop) and authoritative_nf (any provider NotFound). Skipped no longer blocks marking.
2. Chain-end rule: mark not_found iff !resolved && !deferred && !hard_transient && authoritative_nf.
3. pending.rs: refactor drain_once to take providers param (inject mocks in tests; run_pending_drain passes [Apple,Wigle]).
4. Post-pass: increment attempts for every drained BSSID (deleted rows no-op) so all-transient rows age out via delete_expired_pending and ORDER BY attempts ASC rotates fairly.
5. Change drain on_network_error IncrementPending->Ignore (avoid double-increment); remove now-dead IncrementPending variant + match arm.
6. Tests: AC1 [Apple-NF,WiGLE-Skipped]->marked+deleted; twin [Skipped,Skipped]->not marked, pending; AC2 age-out; AC3 >10 rows rotate; AC4 quota-flavored Skipped. Keep existing transient tests green.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented in commit e6da573 (whereamid/src/resolver.rs + pending.rs).
- Split transient_error into hard_transient (NetworkError/HardStop) + authoritative_nf (provider NotFound). Skipped feeds neither -> abstention.
- Chain-end mark iff !resolved && !deferred && !hard_transient && authoritative_nf. All-Skipped BSSID (no provider consulted) stays pending, never falsely marked.
- drain_once now takes injected providers (real Apple never Skips + makes net calls, so mocks required); bumps attempts once per drained-still-pending BSSID post-pass. Ages out via delete_expired_pending; ORDER BY attempts ASC now rotates.
- Removed NetErrorAction::IncrementPending (dead after drain policy -> Ignore) + its match arm.
Tests (whereamid bin binary): drain_chain_marks_not_found_when_apple_nf_and_wigle_skipped (AC1), drain_chain_all_skipped_leaves_pending_unmarked (AC1 twin), drain_chain_marks_not_found_when_apple_nf_and_wigle_quota_exhausted (AC4), drain_once_ages_out_perpetually_unresolved_row (AC2), drain_once_rotates_across_batches_no_head_of_line_monopoly (AC3). Existing transient-suppression tests still green.
Gate: lib 78 / bin 121 / proptests 11 passed; clippy -D warnings clean; cargo fmt --check clean.

Review gate (qa-test-runner + mped-architect, both GO): all new tests independently confirmed to bite via revert-analysis; drain tests 34/34 runs, full suite 3x green. Architect found one latent pre-existing gap this task exposed: the stopped-provider shadow (WiGLE HardStop on B1 leaves B2..Bn unprotected - Apple-NF alone marked them). Fixed in-thread post-review (369429a): stopped[idx] now feeds hard_transient for shadowed BSSIDs; pinned by drain_chain_hard_stop_shadow_does_not_mark_later_bssids. The quota-day 30d-mark tradeoff stands as designed (ACs demand it); task-0128 remains the durable fix.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Provider Skipped (WiGLE unconfigured / quota exhausted) was conflated with genuinely-transient failures, so credential-free mode never marked not_found: every scan re-ran an Apple WPS call per unknown AP, pending rows never aged out, and >=10 immortal rows monopolised every drain.

Changes:
- resolver.rs: replaced the single transient_error set with hard_transient (NetworkError/HardStop) + authoritative_nf (a provider said NotFound). Chain-end marks not_found iff !resolved && !deferred && !hard_transient && authoritative_nf. Skipped now feeds neither set (abstention), so an Apple-definitive miss with WiGLE skipped is recorded while an all-Skipped BSSID (no provider consulted) stays pending.
- pending.rs: drain_once takes an injected provider list (mocks in tests; the real Apple makes net calls and never Skips) and bumps attempts once per drained-still-pending BSSID post-pass, so hopeless rows age out via delete_expired_pending and ORDER BY attempts ASC rotates fairly.
- Removed NetErrorAction::IncrementPending (dead once drain on_network_error moved to Ignore) and its match arm.

Tests: Apple-NF+WiGLE-Skipped marks+deletes; all-Skipped stays pending; quota variant (AC#4); drain_once age-out (AC#2) and batch rotation with >10 stuck rows (AC#3). Existing transient-suppression tests unchanged.

Risk: an Apple-definitive miss with WiGLE merely skipped is now marked not_found for the TTL even though a configured WiGLE might later know it. Deliberate per the ACs; per-provider granularity is deferred to task-0128.
<!-- SECTION:FINAL_SUMMARY:END -->
