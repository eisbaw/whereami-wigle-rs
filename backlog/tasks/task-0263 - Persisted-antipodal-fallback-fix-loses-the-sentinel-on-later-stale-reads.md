---
id: TASK-0263
title: Persisted antipodal-fallback fix loses the sentinel on later stale reads
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 21:36'
updated_date: '2026-07-09 05:33'
labels:
  - history
  - wire-format
dependencies:
  - TASK-0127
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From task-0127 review (mped LOW-2). task-0127 made the fresh locate response distinguish the antipodal-fallback (accuracy_m=1000 sentinel) via a response-only fallback bool. But when a CurrentTrilat fallback position WINS and is persisted, new_fix stores only accuracy_m=1000 into LastFix (which has no fallback field). Served LATER as a stale last-fix, it reports accuracy_m~=1000 (+drift), fallback=false — re-introducing the exact in-band ambiguity 0127 removed, but on the STALE path. Out of 0127 AC-5 scope; the accuracy_m=1000 persistence is pre-existing, and stale fixes already carry stale=true + drift-inflated accuracy, so it is partially mitigated. Fix direction: either do NOT persist an antipodal-fallback fix as an authoritative last_fix, OR add a persisted fallback marker to LastFix so the stale path can echo it. LOW.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Decision recorded: don't-persist-fallback-fix OR add-persisted-LastFix-marker OR accept-as-documented (near-theoretical since fallback is near-always-false per task-0262, and stale fixes already carry stale=true)
- [x] #2 Chosen resolution implemented: if don't-persist, an antipodal-fallback fix (fallback=true / accuracy_m=1000 sentinel) does NOT overwrite the authoritative last_fix, with a test asserting the prior last_fix survives; the stale path can no longer echo accuracy_m~=1000 with fallback=false
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Confirm pos.fallback is available at the CurrentTrilat persist site (server.rs handle_locate).
2. DB: extract insert_history_row helper; add Database::persist_history_only + DbHandle wrapper that records the fixes timeseries WITHOUT upsert_last_fix.
3. server.rs: in CurrentTrilat branch, when pos.fallback, record history-only and do NOT advance last_fix (memory + disk). Reuse the autolocate applied=false -> *last=previous seam.
4. Test: antipodal-fallback fix leaves prior last_fix intact + history records the 1000m observation. Non-fallback update already pinned by existing tests.
5. Verify e2e (baseline 871) + fmt.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DECISION: don't-persist-fallback-fix (the recommended clean path). Commit 5e5b761.

The fallback signal IS cleanly available at the persist site: pos.fallback (trilaterate Position) is in scope in handle_locate's PlausibilityWinner::CurrentTrilat arm (whereamid/src/server.rs), the same site that builds new_fix: LastFix and calls the persist helpers.

Exact writes guarded (CurrentTrilat winner only):
- Interactive path: when pos.fallback, skip persist_locate_fix_and_schedule_locked / persist_last_fix_and_history_locked (which upsert last_fix + update in-memory last_fix + write history) and instead call the new db.persist_history_only. `last` mutex is left untouched so the prior last_fix survives in memory AND on disk. No geocode scheduled (a sentinel position has no meaningful address).
- Autolocate path: when pos.fallback, call persist_history_only and set last_fix_applied=false, reusing the existing applied=false -> *last=previous.clone() seam (same seam the monotonic-drop case uses), so memory realigns to the prior authoritative fix.

FIX-HISTORY still records: added Database::persist_history_only (+ DbHandle wrapper) that INSERTs the fixes-timeseries row WITHOUT upsert_last_fix, via the extracted insert_history_row helper shared with persist_fix_and_history. So the observation is still logged; only the authoritative last_fix (served on the stale path) is skipped.

Non-fallback fix still updates last_fix: unchanged — non-fallback takes the original persist_*_and_history path (last_fix upsert + memory + history). Pinned by existing tests e.g. suspect_current_winner_preserves_address_last_fix_and_history_semantics.

New test: antipodal_fallback_fix_does_not_overwrite_last_fix (server::dispatch_tests) — seeds a prior last_fix at (0,180), pushes two equal-weight antipodal cached APs (guard fires), asserts response fallback=true + accuracy 1000, asserts get_last_fix() is byte-for-byte the PRIOR fix (same at/accuracy/lat/lon/address/sources), and asserts history has exactly 1 row at accuracy 1000 (also proves the CurrentTrilat fallback path was taken, since a last_fix winner writes no history).

No existing test changed: the only prior fallback test (locate_antipodal_cached_aps_report_fallback_true) asserts the wire response only, not last_fix, so it still passes.

No LastFix schema column / migration added (avoided per task guidance). Limit: the stale-path ambiguity was only reachable when fallback=true wins AND is later served stale — near-theoretical per task-0262 — but the fix is clean and cheap, so implemented rather than merely documented.

Gate: `just e2e` 872 passed / 0 failed, clippy -D warnings clean, cargo fmt --check clean.

## Review gate (orchestrator, independent) — GO/GO + hardening
- DECISION: don\x27t-persist (clean path taken). An antipodal-fallback fix (pos.fallback=true, accuracy_m=1000 sentinel) winning via CurrentTrilat no longer overwrites the authoritative last_fix, but STILL records the fixes history timeseries.
- Reviewer CONFIRMED all invariants: (1) both persist arms (interactive server.rs:2021 + autolocate 1964) guard on pos.fallback, skip upsert_last_fix, keep the prior last_fix (memory+disk); (2) the new insert_history_row helper is byte-for-byte the same INSERT INTO fixes (10 cols, same params) — behavior-preserving; (3) fix-HISTORY still records on fallback (persist_history_only inserts the row, skips only upsert_last_fix — no last_fix side effect, routed via the same writer actor, no ordering race); (4) non-fallback path is the unmodified original (still updates last_fix); (5) the new test bites (rows.len()==1 proves the CurrentTrilat path was hit — a last_fix winner writes zero history; stored.at==previous.at catches a sentinel-cache regression). No existing test changed.
- HARDENING (commit 4c2dcba) for the reviewer LOW: added a comment warning that the interactive fallback arm relies on *last not being mutated earlier in the branch (autolocate re-assigns explicitly; interactive relies on it staying the prior value) — prevents a future-edit footgun.
- Dual gate GO/GO (872).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Fixed the stale-path in-band ambiguity task-0127 removed on the fresh path: a winning antipodal-FALLBACK fix (accuracy_m=1000 "position unknown" sentinel) no longer overwrites the authoritative last_fix, so it can never later be served on the stale path as a ~1000 m position with fallback=false.

Implementation (don\x27t-persist): both CurrentTrilat persist arms (interactive + autolocate) branch on pos.fallback; on fallback they call the new Database::persist_history_only (which inserts the fixes history row via the extracted, behavior-preserving insert_history_row helper but skips upsert_last_fix), leaving the prior last_fix intact in memory and on disk. The fix-history timeseries still records the observation; only the authoritative last_fix write is skipped. A non-fallback fix updates last_fix exactly as before. No LastFix schema column/migration added.

Test: antipodal_fallback_fix_does_not_overwrite_last_fix asserts fallback=true/accuracy_m=1000, last_fix byte-identical to the prior fix, and exactly one history row (which also proves the CurrentTrilat fallback path was exercised).

Near-theoretical (fallback near-always-false per task-0262) but the fix is clean + cheap. Reviewer-hardened with a comment guarding the interactive arm\x27s implicit *last invariant. Dual gate GO/GO (872). Commits 5e5b761 + 4c2dcba.
<!-- SECTION:FINAL_SUMMARY:END -->
