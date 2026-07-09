---
id: TASK-0118
title: Replace ChainPolicy 10-field matrix with ChainMode enum
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:14'
updated_date: '2026-07-09 07:16'
labels:
  - refactor
  - resolver
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/resolver.rs:114-133. ChainPolicy has 10 fields incl 3 *Action enums (SkipAction, NetErrorAction, HardStopAction) each carrying QueuePending* variants overlapping with delete_pending_on_success/delete_pending_on_not_found. NotFoundPolicy::Never is #[allow(dead_code)]. The three real flows (readonly, background, drain) are spelled out twice each (production + test helpers). Type permits ~144 syntactically legal policies; ~6 are semantically valid. task-0052 collapsed two illegal pairs but the broader space remains.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Replace ChainPolicy with enum ChainMode { ReadOnly, Background, Drain }
- [x] #2 resolve_chain dispatches per-outcome on the mode (one match arm per mode in each decision site)
- [x] #3 Existing three call sites pass ChainMode::ReadOnly | Background | Drain instead of a ChainPolicy literal
- [x] #4 test_state helpers replaced with the three enum variants
- [x] #5 All existing resolver tests pass
- [x] #6 Doc-comment on ChainMode names each mode's behavior shorthand (cache-only, write-through, drain-with-cleanup)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Build field->mode table from readonly_policy/background-inline/drain (done during onboarding).
2. Replace ChainPolicy struct + 4 field-enums (SkipAction/NetErrorAction/HardStopAction/NotFoundPolicy) with pub enum ChainMode { ReadOnly, Background, Drain } + Copy + doc-comment + boolean mode-methods returning the exact per-mode field value.
3. Thread ChainMode (by value, Copy) through resolve_chain/_with_observations/_inner; replace each policy.<field> read with mode.<method>().
4. 3 call sites: resolve_readonly->ReadOnly, resolve_background->Background, pending drain->Drain.
5. Tests: permissive_policy (a synthetic 4th combo) -> ChainMode::Background (closest real mode; no assertion depends on its unique on_* actions); drain_policy->Drain; rewrite resolve_readonly_policy_is_never to assert !ChainMode::ReadOnly.persists_not_found().
6. Gate: nix develop -c just e2e + cargo fmt --check. No assertion values changed.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0189/task-0092: the ChainPolicy matrix shrank, easing the ChainMode collapse. task-0189 removed NetErrorAction::IncrementPending (drain now bumps attempts once per pass post-resolve in drain_once, not per network error). task-0092 removed NotFoundPolicy::PerProvider (resolve_readonly -> Never; the immediate per-provider not_found write is gone). Remaining enums: NotFoundPolicy = {AtChainEnd, Never}; NetErrorAction = {Ignore, QueuePending}. The three live modes to encode as ChainMode:
- readonly: skip_cached+skip_not_found, write_through=false, not_found=Never, on_skipped=NextProvider, on_net=Ignore, on_hard=Stop.
- background: skip both, write_through, AtChainEnd, on_skipped=QueuePending, on_net=QueuePending, on_hard=QueuePendingAndStop.
- drain: skip neither, write_through, delete_pending_on_success + delete_pending_on_not_found, AtChainEnd, on_skipped=NextProvider, on_net=Ignore, on_hard=Stop.
drain_once also gained an injected `providers: &[Provider]` param for testability; keep that seam if refactoring.

task-0118 implemented (commit 4607693). Replaced ChainPolicy + 4 field-enums with enum ChainMode { ReadOnly, Background, Drain } + private boolean mode-methods. Deleted SkipAction/NetErrorAction/HardStopAction/NotFoundPolicy (incl. dead NotFoundPolicy::Never). 3 call sites pass ChainMode variants. permissive_policy (synthetic 4th combo, NOT one of the 3 prod modes) mapped to closest real mode Background; drain_policy->Drain; resolve_readonly_policy_is_never rewritten to assert !ChainMode::ReadOnly.persists_not_found(). NO test assertion value changed (only construction + one type-shape policy assertion re-expressed). Gate: nix develop -c just e2e = 890 passed / 0 failed (baseline 890), clippy -D warnings clean, cargo fmt --check clean. Left In Progress for review.

## Review gate (orchestrator: rigorous architecture review + in-thread verification) — GO
- Gate: 890/890 (count-neutral refactor), clippy 0 -D warnings, fmt clean; deleted types (ChainPolicy/SkipAction/NetErrorAction/HardStopAction/NotFoundPolicy) have ZERO live references (only doc-comment mentions remain) — grep-verified.
- Architecture (static byte-comparison vs the deleted literals): the 8x3 field->mode mapping is BYTE-IDENTICAL to the 3 original policies (all 24 cells match, incl. Background on_skipped=QueuePending). Mode-methods reproduce the table exactly; dispatch preserves logic (match policy.on_* -> if mode.queues_pending_on_*(), persist=mode.persists_not_found()); no inverted/dropped branch. Production behavior identical for the 3 real modes (resolve_readonly->ReadOnly, resolve_background->Background, drain->Drain).
- KEY RISK verified INERT: permissive_policy() was a synthetic 4th combo (NOT a production mode) remapped to Background; Background additionally queues-pending on Skipped/NetworkError/HardStop, but the 2 affected tests assert on state structurally INDEPENDENT of the pending queue (resolve_chain_transient_error_suppresses_not_found_mark reads hard_transient membership; resolve_chain_not_found_set_is_authoritative_only reads the in-memory not_found set; Skipped feeds neither) — the extra pending INSERTs write a table those assertions never read. Arguably an IMPROVEMENT (tests now run under a real mode, not a synthetic combo).
- No behavioral assertion changed — only test construction + one sanctioned type-shape re-expression (resolve_readonly_policy_is_never -> !ChainMode::ReadOnly.persists_not_found(); behavioral twin resolve_readonly_unknown_bssid_does_not_write_not_found untouched).
- LOW (informational, accepted): the permissive->Background remap adds assertion-invisible pending writes in those 2 tests; harmless today, would only matter if a test later asserts pending contents.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced the ~8-field ChainPolicy struct (which permitted ~144 syntactically-legal combos, ~6 semantically valid) with enum ChainMode { ReadOnly, Background, Drain } — the three real resolver flows. Each former field became a private ChainMode method returning that mode\x27s exact value (skips_cached, skips_not_found, writes_through, manages_pending, persists_not_found, queues_pending_on_{skip,network_error,hard_stop}); resolve_chain_inner dispatches via those methods instead of reading policy fields. Deleted ChainPolicy + SkipAction/NetErrorAction/HardStopAction/NotFoundPolicy (incl. the formerly-dead NotFoundPolicy::Never).

Behavior-preserving: the 8x3 field->mode mapping is byte-identical to the 3 original policy literals (reviewer-verified cell-by-cell); no production behavior changed; no behavioral test assertion changed (only construction + one type-shape re-expression). The test-only permissive_policy (a synthetic 4th combo) was remapped to its closest real mode (Background) — the divergence is inert to every affected assertion.

Doc-comment on ChainMode names each mode: ReadOnly=cache-only, Background=write-through, Drain=drain-with-cleanup (owns/deletes the pending row). 890 workspace tests, dual gate GO/GO. Commit 4607693.
<!-- SECTION:FINAL_SUMMARY:END -->
