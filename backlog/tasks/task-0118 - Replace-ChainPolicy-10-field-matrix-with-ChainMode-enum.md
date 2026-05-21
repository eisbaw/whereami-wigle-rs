---
id: TASK-0118
title: Replace ChainPolicy 10-field matrix with ChainMode enum
status: To Do
assignee: []
created_date: '2026-05-20 21:14'
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
