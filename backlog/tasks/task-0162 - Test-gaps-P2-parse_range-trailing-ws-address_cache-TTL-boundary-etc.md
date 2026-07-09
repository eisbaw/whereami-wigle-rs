---
id: TASK-0162
title: 'Test gaps (P2): parse_range trailing ws, address_cache TTL boundary, etc'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:31'
updated_date: '2026-07-09 06:56'
labels:
  - tests
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Bundle of small test gaps Scout flagged at P2 priority — collected here so each gets a single AC:
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 parse_range("7 d") (with internal whitespace) — currently parses as num='7 ', unit='d' and errors; pin the behavior with an explicit unit test
- [x] #2 split_nmcli_fields proptest INCLUDES literal backslashes (current proptests exclude them per proptests.rs:168-170 note)
- [x] #3 address_cache TTL boundary at age==ttl_days exactly: assert get returns None when age >= ttl_days (line address_cache.rs:78 uses >=)
- [x] #4 handle_locate write-through to last_fix + fixes: end-to-end test that issues locate and asserts both rows after (would have caught task-0046 had it existed pre-fix)
- [x] #5 Server MAX_RESOLVE_BSSIDS=256 rejection (server.rs:684): fire 257 BSSIDs at handle_resolve, assert error response
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. AC#1: pin parse_range("7 d") internal-ws -> Err (duration_spec.rs test).
2. AC#2: verify ALREADY-COVERED by split_nmcli_round_trips_with_escapes (proptests.rs) which includes literal backslash in alphabet; cite, no dup.
3. AC#3: add address_cache TTL boundary test age==ttl_days exactly -> None (deterministic, no sleep).
4. AC#4: add cold-boot dispatch Locate write-through: assert last_fix Some AND history_rows len==1; cite suspect_current_winner for current-winner branch.
5. AC#5: add handle_resolve 257-BSSID rejection test (offline, length check first, no provider call).
6. Gate: nix develop -c just e2e, cargo fmt --check, AC#2 proptest 3x. Commit code only.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
task-0162 implementation (test-only, commit d29e6de):
- AC#1 ADDED parse_range_rejects_internal_whitespace (duration_spec.rs). Bites: "7 d" trims to "7 d", splits off unit 'd', num_str="7 " fails i64::parse -> Err naming spec. A parser that trimmed interior ws would regress silently.
- AC#2 ALREADY-COVERED by split_nmcli_round_trips_with_escapes (tests/proptests.rs:330). Its field alphabet [a-zA-Z0-9:\\ ] already INCLUDES literal backslash (task-0226 widened it; comment lines 324-328). split_nmcli_fields_is_total (.* strategy) also feeds backslashes. No duplicate added. Ran proptest 3x: 2 passed each, no flake.
- AC#3 ADDED address_cache_expires_exactly_at_ttl_boundary. Bites the `age_days >= ttl_days` at line 82: inserted_at=now-days(7) => read-side age truncates to exactly 7 => expired. A regression to `>` returns Some and fails. Existing address_cache_expires_after_ttl used 8>7, not the boundary.
- AC#4 ADDED locate_cold_boot_writes_through_to_last_fix_and_history. Cold-boot dispatch Locate (no previous, CurrentTrilat winner, cached APs only) asserts BOTH get_last_fix Some AND history_rows len==1. suspect_current_winner_preserves_address_last_fix_and_history_semantics already covers the current-over-previous branch; the cold-boot first-fix DB write-through was NOT asserted anywhere (cold_boot test checked JSON only). Bites task-0046 write hazard.
- AC#5 ADDED handle_resolve_rejects_more_than_max_bssids. 257 well-formed BSSIDs -> ok:false "accepts at most 256". Length guard precedes validate + resolve_readonly, so fully offline (empty WiGLE creds, no quota). validate_resolve_bssids_rejects_malformed test did NOT hit the length cap.

Gate: nix develop -c just e2e = 890 passed (baseline 886 + 4 new), 0 failed, clippy -D warnings clean. cargo fmt --check clean. No production change. No live daemon / no real resolve. No test revealed a bug.

## Review gate (orchestrator, independent) — GO/GO
- QA (agent + orchestrator in-thread): diff TEST-ONLY (+125, all in #[cfg(test)] modules; production parse_range/AddressCache::get/handle_resolve unchanged); 890/890 both runs; clippy 0 -D warnings; fmt clean; no flake.
- Architecture: each ADDED test BITES — #1 asserts the specific Err message ("invalid range \x277 d\x27") + a second interior-space case; #3 pins age==ttl exactly (deterministic inserted_at seam, no sleep; regression >=->> returns Some -> fails), distinct from the sibling age>ttl test; #4 drives a real cold-boot Locate dispatch with cached APs (offline) and asserts BOTH get_last_fix Some AND one fixes history row at the DB level (cold-boot first-fix branch, additive to the previous-row-exists sibling); #5 fires 257 BSSIDs at handle_resolve, the length guard (server.rs:2212) fires BEFORE validate_resolve_bssids/resolve_readonly -> ok:false "at most 256", fully OFFLINE (no provider/quota/socket). #2 ALREADY-COVERED confirmed TRUE: split_nmcli_round_trips_with_escapes regex [a-zA-Z0-9:\\ ] genuinely generates a literal backslash (widened by task-0226; the task\x27s "excludes backslashes" note is stale) + full round-trip invariant. No vacuous assertions; no live-daemon/quota contact.
- Two LOW (accepted): (a) AC#5 tests only 257-over-cap, not 256-accepted — the exact boundary is NOT safely testable offline (256 valid BSSIDs would pass the length guard and proceed to the Apple provider path, burning quota), so the >->>= edge is deliberately unpinned. (b) AC#4 comment "previously only JSON-asserted" is context-correct for the cold-boot path (the sibling covers DB-level for the non-cold branch); new cold-boot coverage is genuine.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Filled 5 P2 test gaps (test-only, +125 lines):
- AC#1 parse_range_rejects_internal_whitespace: "7 d" -> num_str "7 " fails i64::parse -> Err; pins the behavior (a future interior-whitespace-trimming parser would fail the test).
- AC#2 (already covered): split_nmcli_round_trips_with_escapes already generates literal backslashes (task-0226 widened the alphabet) with a full round-trip invariant — no duplicate added.
- AC#3 address_cache_expires_exactly_at_ttl_boundary: age==ttl exactly returns None (the >= boundary the old age>ttl test missed); deterministic inserted_at seam.
- AC#4 locate_cold_boot_writes_through_to_last_fix_and_history: a cold-boot Locate (empty tables, cached APs, offline) asserts BOTH a last_fix row AND a fixes history row at the DB level — the task-0046 hazard, previously only asserted for the non-cold branch.
- AC#5 handle_resolve_rejects_more_than_max_bssids: 257 BSSIDs -> ok:false "at most 256", the length guard firing before any provider call (offline, no quota).

No production change; no live daemon; no real resolve. Dual gate GO/GO (890, no flake). Accepted limitations: the 256-accepted boundary is not testable offline (would hit the provider path). Commit d29e6de.
<!-- SECTION:FINAL_SUMMARY:END -->
