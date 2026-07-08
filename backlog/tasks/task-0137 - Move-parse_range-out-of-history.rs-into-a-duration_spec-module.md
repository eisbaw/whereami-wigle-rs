---
id: TASK-0137
title: Move parse_range out of history.rs into a duration_spec module
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:22'
updated_date: '2026-07-08 20:37'
labels:
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/history.rs:124-150. parse_range parses s/m/h/d/w duration specs — generic, not history-specific. If task-0031's history feature ever gets siblings (e.g. stats-over-time endpoint), parse_range gets pulled out, breaking history.rs's pub API. Better to extract while there is one caller.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 New module src/duration_spec.rs (or in src/geo.rs alongside haversine helpers — author choice)
- [x] #2 parse_range moved; history.rs uses 'use crate::duration_spec::parse_range'
- [x] #3 Existing tests in history.rs (including parse_range_no_panic_on_non_ascii from task-0048) move with the function
- [x] #4 Kani harness for parse_range_no_panic (task-0082) moves too
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Create whereamid/src/duration_spec.rs; add mod duration_spec to main.rs (bin-only).
2. Move parse_range verbatim as pub(crate) with anyhow and chrono imports.
3. Move the 4 parse_range unit tests into duration_spec tests mod.
4. Move the cfg(kani) parse_range_no_panic harness into duration_spec.
5. GOTCHA: prompt wrong - server.rs calls crate history parse_range twice (lines 2763, 2786). Repoint to crate duration_spec parse_range. history.rs has no production caller, so do NOT add a use import (unused under -D warnings); trim history.rs imports instead (drop anyhow line, drop TimeDelta and Utc from chrono, keep DateTime).
6. Verify: nix develop -c just e2e; cargo fmt --check; grep parse_range.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- Extracted parse_range to whereamid/src/duration_spec.rs (pub(crate)); mod added to main.rs (bin-only).
- GOTCHA: task premise wrong - server.rs had 2 callers via crate::history::parse_range (lines 2763, 2786); repointed to crate::duration_spec. history.rs had NO production caller, so no use-import added (would be unused under -D warnings); trimmed history.rs imports (dropped anyhow line + TimeDelta/Utc from chrono).
- Moved 4 parse_range unit tests + cfg(kani) parse_range_no_panic harness verbatim.
- Gate: just e2e PASS (whereamid 464 passed/0 failed; proptests 12; shutdown 4; clippy -D warnings clean); cargo fmt --check clean; cargo test parse_range -> 4 duration_spec::tests ran, 0 failed. Kani not runnable here (broken toolchain) - harness verified statically only.
- Commit 0d53bcb (code only). Left In Progress for review gate.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Extracted parse_range (s/m/h/d/w duration-spec parser) out of history.rs into a dedicated whereamid/src/duration_spec.rs module (commit 0d53bcb) — it is generic, not history-specific, so a future sibling feature can reuse it without breaking history.rs's API. Pure relocation, no behavior change.

Changes: new duration_spec.rs (mod in main.rs, bin-only like history) holding pub(crate) parse_range + its 4 unit tests + the #[cfg(kani)] parse_range_no_panic harness; history.rs loses the fn/tests/kani-mod and its now-unused imports; server.rs's 2 call sites repointed from crate::history::parse_range to crate::duration_spec::parse_range. Chose pub(crate) (only crate-internal consumers).

PREMISE CORRECTION (implementer caught, good delegation outcome): the task claimed parse_range had no caller outside history.rs — it actually had TWO in server.rs (handle_history range parsing). Both were repointed; history.rs got NO leftover import (it has zero remaining uses), which -D warnings would have flagged.

Verification: pure relocation, orchestrator-verified: parse_range now defined only in duration_spec.rs + used via the module path in server.rs (no dangling ref); the 4 moved tests run + pass (parse_range filter: 4 passed); whereamid 464 unit + 12 proptest + 4 shutdown, clippy -D warnings + fmt clean, e2e green. AC#4 kani harness moved verbatim with #[cfg(kani)]/#[kani::proof]/#[kani::unwind(33)] intact — statically compile-checked only (Kani unrunnable in this env per project memory; not machine-verified, pre-existing limitation).
<!-- SECTION:FINAL_SUMMARY:END -->
