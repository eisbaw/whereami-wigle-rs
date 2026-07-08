---
id: TASK-0116
title: Slice handle_locate (247 lines) into testable units
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:13'
updated_date: '2026-07-08 23:14'
labels:
  - refactor
  - server
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:397-643. Phases: stable candidate selection (411-417), cold-start fallback (419-436), cached lookup (457-476), trilateration (488), address resolution (501-567), persistence (585-613), response build (615-630). Each phase is testable in isolation; none currently is. task-0058 added ok_json helper but the handler kernel wasn't sliced.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Extract handle_locate_inner(state, candidates: &[(String,i32)]) -> Result<Position> — the pure kernel: cached lookup + filter + trilat. No mutex acquisition, no spawning, no DB writes.
- [x] #2 Extract persist_locate(state, position, sources) -> Result<()> — last_fix upsert + fixes insert. Single lock-ordering site.
- [ ] #3 Extract resolve_cold_start_fallback(state) -> Vec<(String,i32)> — for the empty-candidates case
- [x] #4 handle_locate becomes a thin assembler over the three (~30 lines)
- [x] #5 New unit tests: handle_locate_inner with synthetic candidates returns expected Position without touching the DB beyond cache reads
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Confirm AC#2 (persist_locate_fix_and_schedule[_locked]) and AC#4 (thin handle_locate over compute_locate) already satisfied by prior refactors; note in AC.
2. AC#1: extract PURE kernel solve_locate_from_cache(cached_aps, active, top_n) -> LocateKernel enum {NoPositions, Solved{result, visible_cached, source_count, cached_count}} doing the finite/mobile filter + top-N membership + consensus denominator + trilaterate. No mutex/spawn/DB-write. Cache READ stays in caller (interactive touches last_seen; auto read-only) — the one impure step deliberately excluded.
3. Rewire compute_locate_with_clock to call the kernel; keep cold-start + Ok/Err arms byte-identical. Move trilaterate/PositionedAp import into kernel to avoid unused-import.
4. AC#3: assess using_fallback/active_candidates selection — 2-line inline expr, already clear (+ bounded_uncached_observations already extracted). Note rather than force awkward reference-returning fn.
5. AC#5: unit test seeding cached APs (cache_ap) + synthetic Candidate map -> assert Position via kernel, no DB write/spawn/last_fix.
6. Gate: nix develop -c just e2e + cargo fmt --check. Full dispatch suite must pass unchanged.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED (commit dfcb7a4) — left In Progress for review gate; ACs intentionally NOT checked.

- AC#1 (pure kernel): DONE. Extracted solve_locate_from_cache(cached_aps, active, top_n) -> LocateKernel{NoPositions | Solved(SolvedLocate{result, visible_cached, source_count, cached_count})}. Finite/mobile filter + top-N membership + task-0094 consensus denominator (same pass) + trilaterate. No mutex/spawn/DB-write. Cache READ stays in caller (interactive touches last_seen; autolocate read-only) — the one impure step deliberately excluded.
- AC#2 (persist): already satisfied by persist_locate_fix_and_schedule[_locked] (server.rs:312/365). Confirmed, not redone.
- AC#3 (cold-start fallback): the using_fallback / active_candidates selection is a 2-line inline expr referencing local Vecs; extracting a fn would force cloning or borrowed-ref returns (less clear, risk of behavior change). The bounded cold-start provider work is ALREADY its own fn (bounded_uncached_observations, server.rs:1244). Left inline as clearer; noted.
- AC#4 (thin handle_locate): already satisfied — handle_locate just match compute_locate(Interactive) + format. Confirmed, not redone.
- AC#5 (unit tests): DONE. Two pure #[test]s in dispatch_tests: expected-Position from synthetic rows; eligibility/consensus/top-N invariants. No DB/mutex/spawn.

GATE (nix develop -c just e2e + cargo fmt --check): PASS. whereamid main bin 470 passed (was 468; +2 new). dispatch_tests suite 74 passed. Workspace total 789 passed / 0 failed. clippy --all-targets -D warnings clean. fmt --check clean.

Reviews: mped-architect (byte-identical, consensus same-pass invariant preserved, no moved side effects) + qa-test-runner (789/0) both green.

Gotcha: source_count == cached_count (both positioned_aps.len()) is pre-existing duplicated derived state, faithfully preserved (out of scope for a behavior-preserving change).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Extracted a pure, unit-testable trilateration kernel from compute_locate (commit dfcb7a4, server.rs only). Behavior BYTE-IDENTICAL — the full 74-test dispatch/locate suite is unchanged.

New fn solve_locate_from_cache(cached_aps: &[ApInfo], active: &HashMap<&str,&Candidate>, top_n) -> LocateKernel (NoPositions | Solved(SolvedLocate{ result: Result<Position>, visible_cached, source_count, cached_count })): the finite/mobile eligibility filter + top-N solver membership + task-0094 consensus denominator (visible_cached, built SAME-pass/same-order as positioned_aps) + trilaterate. No mutex, no spawn, no DB write (deterministic-in-inputs; it does emit the task-0227 non-finite warn! — a tracing side effect, documented).

AC status: #1 pure kernel extracted DONE. #2 persistence-extracted + #4 thin-assembler ALREADY satisfied by prior refactors (persist_locate_fix_and_schedule[_locked]; handle_locate is a thin match over compute_locate) — confirmed, not redone. #3 cold-start fallback fn NOT extracted (deliberate, documented): the using_fallback/active_candidates selection is a trivial inline expr over borrowed Vecs; extracting it would force clones or borrowed-ref returns with real behavior-change risk, and the bounded cold-start provider work is already its own fn (bounded_uncached_observations). #5 two pure unit tests DONE (locate_kernel_returns_expected_position_from_synthetic_rows, locate_kernel_filters_ineligible_rows_and_respects_top_n — synthetic ApInfo/Candidate, no DB; cover the eligibility filter, top_n cap, consensus-denominator-vs-membership distinction, and NoPositions).

DESIGN: the impure cache READ stays in the caller because interactive locate touches last_seen while autolocate is read-only (lookup_cached vs lookup_cached_readonly) — the read is genuinely impure and belongs above the kernel. LocateKernel/SolvedLocate is justified (the two variants drive disjoint caller policy: cold-start resolve vs trilat outcome; SolvedLocate carries the consensus denominator a bare Result<Position> could not).

Verification: dual gate. qa GO (789 tests; dispatch suite 74/74 UNCHANGED + deterministic x2; 2 kernel tests bite; clippy -D warnings + fmt clean). mped GO — traced compute_locate before/after and confirmed the kernel loop is character-for-character the old inline loop re-homed, visible_cached still built same-pass/same-order (consensus denominator intact), the one count-reordering is provably harmless (unread in the empty branch), impurity boundary clean, task-0248 non-finite warn test still bites the kernel path. 389-line diff is mostly the 2 tests + heavy accurate doc-comments + verbatim re-indent of the cold-start block into a match arm — little restructuring beyond the seam. source_count==cached_count preserved (pre-existing, tracked by task-0261).
<!-- SECTION:FINAL_SUMMARY:END -->
