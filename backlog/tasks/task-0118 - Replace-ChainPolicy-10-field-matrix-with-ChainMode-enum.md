---
id: TASK-0118
title: Replace ChainPolicy 10-field matrix with ChainMode enum
status: To Do
assignee: []
created_date: '2026-05-20 21:14'
updated_date: '2026-07-02 18:19'
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
- [ ] #1 Replace ChainPolicy with enum ChainMode { ReadOnly, Background, Drain }
- [ ] #2 resolve_chain dispatches per-outcome on the mode (one match arm per mode in each decision site)
- [ ] #3 Existing three call sites pass ChainMode::ReadOnly | Background | Drain instead of a ChainPolicy literal
- [ ] #4 test_state helpers replaced with the three enum variants
- [ ] #5 All existing resolver tests pass
- [ ] #6 Doc-comment on ChainMode names each mode's behavior shorthand (cache-only, write-through, drain-with-cleanup)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0189/task-0092: the ChainPolicy matrix shrank, easing the ChainMode collapse. task-0189 removed NetErrorAction::IncrementPending (drain now bumps attempts once per pass post-resolve in drain_once, not per network error). task-0092 removed NotFoundPolicy::PerProvider (resolve_readonly -> Never; the immediate per-provider not_found write is gone). Remaining enums: NotFoundPolicy = {AtChainEnd, Never}; NetErrorAction = {Ignore, QueuePending}. The three live modes to encode as ChainMode:
- readonly: skip_cached+skip_not_found, write_through=false, not_found=Never, on_skipped=NextProvider, on_net=Ignore, on_hard=Stop.
- background: skip both, write_through, AtChainEnd, on_skipped=QueuePending, on_net=QueuePending, on_hard=QueuePendingAndStop.
- drain: skip neither, write_through, delete_pending_on_success + delete_pending_on_not_found, AtChainEnd, on_skipped=NextProvider, on_net=Ignore, on_hard=Stop.
drain_once also gained an injected `providers: &[Provider]` param for testability; keep that seam if refactoring.
<!-- SECTION:NOTES:END -->
