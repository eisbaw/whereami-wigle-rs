---
id: TASK-0141
title: 'history segment_fixes: surface geometric_median Capped result'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:24'
updated_date: '2026-07-08 20:23'
labels:
  - observability
  - history
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/history.rs:84-110. Calls geometric_median with .unwrap_or((cur[0].lat, cur[0].lon)) on None. But geometric_median (thin adapter) collapses Capped|Converged into Some, Antipodal|Degenerate into None. So if Weiszfeld hit the iteration cap with a not-yet-converged iterate, segmentation uses it without warning. The wrong iterate could shift the segment-membership decision (member vs new-segment) silently.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 history::segment_fixes calls geometric_median_detailed (the rich variant from task-0085), not the thin Option adapter
- [x] #2 Match on Converged: use it. Capped: tracing::debug log including the iterate and seg_id, use it (best effort). Antipodal/Degenerate: tracing::warn, fall back to cur[0]
- [x] #3 geometric_median_detailed becomes pub(crate) so history.rs (binary crate) can reach it (this either depends on task-0112 OR uses crate-local path)
- [x] #4 Test: history with a deliberately-pathological cluster that triggers Capped — verify a debug log fired (custom subscriber)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Split match arms in compute_geometric_median (history.rs): Converged->Point (no log); Capped->tracing::debug (point + coords.len(), best-effort note) then Point; Antipodal/Degenerate->tracing::warn (variant + coords.len(), fallback) then Degenerate; Cancelled unchanged. Add code comment re seg_id unavailable at this layer.
2. Empirically find a coord cluster that returns GeoMedianResult::Capped (probe with real geometric_median_detailed).
3. Add test: install_permissive_global_test_subscriber, scoped CapturedEvents sink, drive compute_geometric_median with Capped cluster, assert DEBUG event mentioning cap. Add Degenerate warn test (antipodal/empty).
4. Gate: cargo test -p whereamid, just e2e, clippy -D warnings, fmt --check. Run new tests twice.
5. Commit code only, no attribution, conventional commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward update from TASK-0087 (2026-07-05): history no longer uses the thin Option median adapter in production and no longer computes a median for membership. It calls the detailed cancellation-aware median once at retained-segment flush. A Capped iterate therefore cannot change membership anymore; this task remains an observability task only for the reported final centroid and Antipodal/Degenerate fallback. TASK-0142 is a duplicate with the same narrowed scope.

task-0142 was a verbatim duplicate of this task (same src location, ACs, intent; filed one minute apart) — archived it, this is the canonical one (task-0141).

Implemented AC#2 + AC#4 (commit 43ecb59).

- Split compute_geometric_median match arms: Converged->Point (no log); Capped->tracing::debug (?point, coord_count, best-effort note) then Point; Antipodal & Degenerate->tracing::warn (variant + coord_count, fallback note) then Degenerate; Cancelled unchanged. Behaviour identical, logging added only.
- seg_id decision: NOT threaded. compute_geometric_median is an injected MedianComputer fn taking only (coords, is_cancelled); rethreading seg_id through the trait + test double is invasive/out of proportion. Logged coord_count instead + code comment documents this.
- Added MedianComputation derive(Debug) for test assertions.
- Tests (history::tests::median_observability): capped_median_emits_debug_and_still_returns_point + degenerate_median_emits_warn_and_falls_back. Scoped CapturedEvents sink mirroring db.rs; install_permissive_global_test_subscriber() first (task-0251).
- HOW Capped triggered: cluster [(0,0),(5,20),(-5,10),(2,15)] — geometric median coincides with the (2,15) input point, so Weiszfeld converges too slowly (singular weight near data point) and exhausts WEISZFELD_MAX_ITERS=50. Found empirically via throwaway probe against the REAL geometric_median_detailed (probe removed before commit). Last iterate ~(2.0,15.0).

GATE (all via nix develop -c): cargo test -p whereamid = 204+464+12+4 passed, 0 failed. new tests run twice, stable. clippy --all-targets -D warnings = exit 0 (only dirty-tree note). fmt --check = clean. just e2e = exit 0 (all suites pass).

Left In Progress for external review gate. AC#1/#3 already done by prior refactor; not re-touched.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Made the geometric-median outcome observable in history segmentation (commit 43ecb59, whereamid/src/history.rs). Previously compute_geometric_median collapsed Converged|Capped -> Point and Antipodal|Degenerate -> Degenerate SILENTLY, so a Weiszfeld iteration that hit the cap without converging (Capped) drove the segment-membership decision with no signal.

Change (observability-only, behavior IDENTICAL — outcome mapping unchanged): split the match arms so Capped emits tracing::debug! (point + coord_count, best-effort note) then still returns Point; Antipodal and Degenerate each emit tracing::warn! (variant + coord_count) then still fall back to Degenerate; Converged/Cancelled unchanged. AC#1 (uses the detailed geometric_median_detailed_cancellable variant) and AC#3 (GeoMedianResult pub(crate) reachable) were ALREADY satisfied by a prior refactor and left untouched; this cycle delivered the missing AC#2 (logging) + AC#4 (test).

SEG_ID: not threaded into the log — compute_geometric_median is an injected MedianComputer fn receiving only (coords, is_cancelled); rethreading seg_id through the trait + counting test-double is invasive and out of proportion, so coord_count is logged with a code comment noting seg_id is unavailable at this layer.

Tests (AC#4): median_observability submodule (scoped CapturedEvents + crate::test_support::install_permissive_global_test_subscriber() from task-0251 for reliable capture): a Capped cluster [(0,0),(5,20),(-5,10),(2,15)] (median lands on a data point -> singular Weiszfeld weight -> exhausts WEISZFELD_MAX_ITERS -> Capped) asserts a DEBUG "iteration cap" event; an antipodal pair asserts Degenerate + a WARN "degenerate" event.

Verification: observability-only, behavior-preserving diff (git-confirmed: outcome mapping identical, only logging added), orchestrator-verified: new tests pass twice, whereamid 204 lib / 464 bin 0 failed, clippy -D warnings + fmt clean, e2e green.

Housekeeping: task-0142 was a verbatim duplicate of this task (same src, ACs, filed 1 min apart) — archived. Honest limit: the Capped fixture is empirically derived and depends on WEISZFELD_MAX_ITERS=50 / WEISZFELD_CONVERGENCE=1e-8; if those loosen enough that this cluster converges, the debug test fails LOUDLY (Converged, no debug log) rather than silently — a safe failure mode, documented in the test comment.
<!-- SECTION:FINAL_SUMMARY:END -->
