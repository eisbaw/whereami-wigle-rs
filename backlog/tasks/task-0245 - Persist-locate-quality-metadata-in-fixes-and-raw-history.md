---
id: TASK-0245
title: Persist locate quality metadata in fixes and raw history
status: Done
assignee:
  - task0245-implementer
created_date: '2026-07-05 14:23'
updated_date: '2026-07-07 23:10'
labels:
  - observability
  - history
dependencies:
  - TASK-0096
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0096 applies the TASK-0094 plausibility policy to autonomous fixes, but the fixes table and TASK-0095 raw-history shape store only timestamp, coordinates, accuracy, and source count. Add durable nullable quality metadata so both manual and autonomous trajectories can expose the confidence and decision context they actually had at collection time. This is a schema/protocol expansion and is deliberately separate from the autonomous-loop refactor.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Add a transactional schema migration with nullable implied_speed_kmh, last_fix_age_secs, inlier_ratio, confidence, and suspect_reason fields; legacy rows remain readable as unknown quality.
- [x] #2 Persist the selected TASK-0094 decision metadata atomically with every newly appended manual and autonomous fix.
- [x] #3 Extend raw-history daemon/client JSON additively with optional quality fields while preserving legacy segmented history and older-row compatibility.
- [x] #4 Document and render unknown versus present quality without fabricating defaults for legacy rows.
- [x] #5 Add migration, round-trip, legacy-row, manual-fix, autonomous-fix, and raw-wire regression tests.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a neutral daemon quality module containing the single daemon-side Confidence enum and validated FixQuality bundle. All production bundles have confidence; speed is finite/nonnegative and paired with nonnegative age, ratio is finite in 0..=1, and a nonempty reason is present exactly for Suspect. Keep the client mirror because no shared protocol crate exists.
2. Migrate fixes transactionally from schema v6 to literal v7 with five nullable, no-default columns: implied_speed_kmh REAL, last_fix_age_secs INTEGER, inlier_ratio REAL, confidence TEXT, suspect_reason TEXT. Probe every column independently, verify exact type/nullability/default for existing columns, fail actionably on malformed shape, extend v7 shape repair across all five columns, and never repair a future-version schema down.
3. Represent an all-NULL persisted bundle as FixRow.quality=None. Decode any metrics/reason without confidence, invalid vocabulary/value/range/presence pairing, or invalid reason relationship as corruption with row-id context. Require Some(FixQuality) at both manual and autonomous production append APIs and bind all five fields in the original fixes INSERT inside the same IMMEDIATE transaction as last_fix UPSERT. Keep test-only legacy inserts explicitly all-NULL.
4. Make PlausibilityDecision own the exact FixQuality used for both response and persistence. Append only when CurrentTrilat wins: interactive and non-deduplicated autonomous paths persist the exact selected decision. LastFix winner, stale fallback, dedup, admission failures, and address backfill append nothing. A backward-clock current observation still appends its own quality even when the monotonic last_fix UPSERT retains the newer row. Atomic means one SQLite transaction; no second UPDATE or process-memory atomicity expansion.
5. Extend both full-range and capped SQL projections, raw daemon history, and typed client HistoryFix with the five flat optional wire fields. Omit absent keys rather than emitting null. Use Option<Confidence> for history so an old daemon or legacy row remains unknown; preserve byte-stable raw JSON passthrough, closed confidence vocabulary, and the unchanged segmented history shape.
6. Extend human raw-history rows with a stable suffix: [quality=unknown] for an all-NULL legacy row; known rows show quality plus speed, last_age, and inliers using n/a for legitimately undefined metrics, and Suspect adds a sanitized reason. Update protocol, README retention/privacy inventory, PRD schema/version, dataflow-history, and dataflow-persist source plus SVG exports without absorbing unrelated diagram-cleanup work.
7. Add migration lockstep/rollback/retry/partial-repair/future-version tests; legacy/full/boundary/corrupt DB round trips through both range APIs; transaction and commit-count regressions; exact manual/autonomous decision persistence including backward clock and no-write paths; mixed raw-wire/client/renderer compatibility; unchanged segmentation; and VM schema/legacy omission/wipe coverage.
8. Run focused Rust tests, cargo fmt, git diff hygiene, exact-source Nix package/parity/bounds and full VM checks, draw.io structural/GUI/export review, exact just e2e, then mandatory parallel qa-test-runner and mped-architect review. Fix every finding with the same sole writer, re-run frozen gates, close only through backlog CLI, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Preflight was performed at clean source HEAD 3bb2c30 after TASK-0246 passed 709 E2E tests and strict Clippy. A focused baseline migration run passed 11 tests. Parallel read-only QA and architecture audits returned conditional GO after pinning the contracts below.

Unknown versus known is a bundle distinction. All five SQL NULLs mean a legacy or external row with unknown quality. Every new daemon append supplies a validated FixQuality with non-null confidence. Cold boot is still known quality: confidence is present while speed/age can be n/a. No default, COALESCE, or locate-style missing-confidence=High behavior is allowed in history. A partial stored bundle without confidence or any invalid field fails with actionable row context rather than degrading to unknown. Segmentation continues to ignore quality and must remain wire-stable.

Quality belongs to the appended observation, not LastFixRow. This is load-bearing when wall time moves backward: the history INSERT commits the incoming decision while the monotonic last_fix UPSERT can retain a newer authority. Address backfill changes only last_fix.address. Production persistence includes quality in the original INSERT and same transaction, never a follow-up metadata write.

The v7 migration adds nullable columns without defaults so v6 rows remain all-NULL. Existing partial columns are checked for exact declared type, nullability, and absence of defaults before missing columns are added. A falsely stamped v7 resets to v6 only when version is not future; version greater than current is left untouched.

Raw history uses Option<Confidence>, omits absent keys, rejects unknown confidence in typed v1 parsing, and preserves raw byte passthrough. Human format is appended to each existing row: [quality=unknown] for legacy; known rows use [quality=High speed=n/a last_age=n/a inliers=1.00] or concrete metrics; Suspect adds a terminal-sanitized reason. Protocol docs retain the rule that adding a confidence token requires a version bump.

Known documentation debt in scope: PRD still says schema v5 and omits aps.is_mobile; dataflow-persist labels schema_version=5 and old fix columns. Update only the affected history/persistence diagrams and generated SVGs; TASK-0217 remains separate.

## Completion + review gate (orchestrator session 2026-07-08)

Prior implementer session left the work uncommitted with a RED gate; this session finished it under the phase3-backlog-ralph review gate.

**Two gate-blocking issues found and fixed in-thread:**
1. Stale existing test `migrate_repairs_v6_stamp_missing_is_mobile_column` asserted post-migrate stamp==6; with SCHEMA_VERSION now 7 the repaired-down v5 shape re-migrates through v6->v7, so the landing stamp is SCHEMA_VERSION. Updated assertion + comment (intent "is_mobile restored" preserved).
2. `cargo fmt` violations from the prior session — reformatted.

**Flaky test caught by the gate (root-caused, not masked):** the two exact-decision tests compared a bit-exact SQLite-read f64 speed against `json[...].as_f64()`. serde_json's default float parser is not correctly-rounded (`float_roundtrip` off), so the JSON re-parse lands up to 1 ULP off; speed varies run-to-run via centroid float-summation order, so it was ~8% flaky (measured 4/50). Fixed at root with a shared `assert_speed_matches` relative-tolerance helper (~1e-9, ~1e6x ULP). inlier_ratio/age stay exact (deterministic/integer). Verified 60/60 on both tests post-fix.

**Parallel read-only review gate: qa-test-runner GO + mped-architect GO.** In-thread hardening applied from review findings:
- Cross-reference doc comments on BOTH `Confidence` enums (daemon quality.rs + client lib.rs) naming the mirror + task-0114; documents the intended `#[default] High` asymmetry (locate-only; history uses Option so absence=None, never High).
- Invariant comment at the three `evaluate_plausibility` `FixQuality::new(...).expect(...)` sites (speed finite>=0 via haversine clamp; score None or in 0..=1) — the expect now aborts on out-of-domain input, so the guards are liveness-critical.
- Comment on `corrupt_fix_quality`'s synthetic `(10, Type::Text)` ordinal (real context is the row-id message).
- Documented segmented-history blast radius: a corrupt quality column fails the position-only segmented view too (deliberate fail-loud); filed task-0247 for an optional position-only projection.

**Final gate (reliably green, flake fixed): 734 tests pass, clippy -D warnings clean, cargo fmt --check clean.** Confidence serde parity verified (both enums rename_all=lowercase, identical variants/order).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Persist per-fix locate quality metadata (TASK-0094 plausibility decision) durably in the fixes table and additively on the raw-history wire, so manual and autonomous trajectories expose the confidence/motion/consensus context they had at collection time.

## Changes
- **New `whereamid/src/quality.rs`**: closed `Confidence` vocabulary (high/medium/low/suspect) + validated `FixQuality` bundle. Sole production constructor enforces speed/age present-together, speed finite&nonneg, age nonneg, inlier_ratio finite in 0..=1, and suspect_reason nonempty iff Suspect.
- **Schema v6->v7** (`db.rs`): transactional migration adds five nullable no-default columns; per-column independent probe (completes interrupted partial repairs), malformed-shape rejection, v7 shape-repair extended, future-version never repaired down. `FixRow.quality: Option<FixQuality>`; all-NULL legacy row decodes to None (unknown), any partial/invalid bundle is a hard read error carrying row-id context — never silently degrades to unknown.
- **Atomic persistence** (`db.rs`/`server.rs`): the five quality fields bind in the SAME IMMEDIATE transaction as the last_fix UPSERT — one commit, no second write. `PlausibilityDecision` owns a single `FixQuality` used for BOTH the response and the history INSERT (never recomputed). Quality belongs to the appended observation: a backward-clock fix persists its own quality even when the monotonic last_fix retains a newer row.
- **Wire + client** (`whereami-client`): `HistoryFix` gains five optional flat fields; absent keys omitted (never null); `confidence: Option<Confidence>` so an old daemon/legacy row stays unknown and never defaults to High (unlike locate). Human raw-history renderer shows `[quality=unknown]` for legacy, known metrics with `n/a` for undefined, and a sanitized/escaped suspect reason. Raw byte-passthrough preserves unknown tokens.
- **Docs**: protocol.md (additive fields, legacy omission, closed vocabulary, version-bump rule), PRD schema v5->v7 (+ is_mobile), README retention/privacy inventory, nix VM test (legacy+quality fixtures, column-shape/all-NULL/raw-wire/schema-version assertions), dataflow-history/persist diagrams + SVGs.

## Tests
Migration lockstep/rollback+retry/per-column-repair/partial-repair/future-version; legacy/full/boundary/corrupt round-trips through both range APIs (11 corruption cases with row-id context); exact manual/autonomous decision persistence incl. backward-clock and no-write paths; additive raw-wire + old-client compat + unknown-token passthrough; renderer unknown/known/malformed; VM schema/legacy-omission coverage. Root-caused and fixed an ~8% float-equality flake (serde_json default parser not correctly-rounded) via a relative-tolerance helper.

## Gate
734 workspace tests pass, clippy -D warnings clean, cargo fmt --check clean. Parallel read-only review: qa-test-runner GO, mped-architect GO.

## Follow-ups filed
- task-0247 (LOW): optional position-only projection so a corrupt quality column cannot fail the quality-agnostic segmented history (today it fails loudly by design).
- Pre-existing task-0114 tracks extracting a shared protocol crate to remove the daemon/client Confidence mirror.
<!-- SECTION:FINAL_SUMMARY:END -->
