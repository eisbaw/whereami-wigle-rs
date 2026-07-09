---
id: TASK-0126
title: Rssi(i32) newtype tracking measurement source (nmcli linear vs iw dBm)
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:18'
updated_date: '2026-07-09 03:02'
labels:
  - correctness
  - domain
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #5. trilaterate.rs:418 computes weight = 10^(dBm/20). nmcli linear-percent and iw raw-dBm both reach trilaterate as signal_dbm: i32 — the type says they're the same physical quantity but they're not (nmcli's mapping is documented as biased; iw is authoritative). Also weight function has no name — it's not a probability, not a distance, just 'stronger->more vote'. Future contributor will read meaning into 10^(dBm/20) that isn't there.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Introduce #[repr(transparent)] struct Rssi(i32) with Rssi::from_iw_dbm and Rssi::from_nmcli_percent constructors
- [x] #2 Track source via enum RssiSource { IwDbm, NmcliPercentLinearized } stored alongside (PositionedAp gains a rssi_source field, OR Rssi carries it)
- [x] #3 Document at the type level: nmcli-derived values may have systematic bias; iw is authoritative; trilateration may down-weight or refuse to mix sources
- [x] #4 Extract fn vote_weight(rssi: Rssi) -> f64 with a documented name (vote-weight, not 'amplitude' or 'distance proxy')
- [x] #5 Unit test: Rssi::from_iw_dbm(-60).vote_weight() > Rssi::from_iw_dbm(-80).vote_weight()
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. New whereamid/src/rssi.rs: #[repr(transparent)] Rssi(i32) + RssiSource{IwDbm,NmcliPercentLinearized}; from_iw_dbm (identity), from_nmcli_percent (MOVE -90+pct*60/100 here), dbm(); type-level docs (nmcli biased, iw authoritative, future may down-weight/refuse). Wire mod into lib.rs+main.rs.
2. Thread RssiSource scanner->trilateration (in-memory only, wire/DB stay i32): ScannedNetwork+source (parser stamps iw=IwDbm, nmcli=NmcliPercentLinearized; nmcli uses Rssi::from_nmcli_percent().dbm()); ScanEntry+source; Candidate+source; PositionedAp+rssi_source.
3. Refactor vote_weight(Option<Rssi>, fallback)->f64; math UNCHANGED (10^(clamp(dbm)/20)); call sites map signal_dbm->Rssi.
4. trilaterate reads rssi_source via a debug! source-mix log (no dead_code, no behavior change).
5. Tests: rssi.rs pins from_nmcli_percent(100)=-30/(0)=-90; trilaterate AC#5 vote_weight(iw -60)>(iw -80). Script mechanical literal edits.
6. Gate: nix develop -c just e2e + cargo fmt --check. Verify existing trilateration numeric tests pass UNCHANGED.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented Rssi(i32) repr(transparent) + RssiSource in new whereamid/src/rssi.rs (wired into lib.rs+main.rs).

Design note on AC#1 vs AC#2 tension: #[repr(transparent)] mandates a single i32 field, so RssiSource CANNOT be a field inside Rssi. Source is therefore tracked ALONGSIDE, on PositionedAp.rssi_source (the AC#2 option that is compatible with repr(transparent)).

nmcli linearization (-90 + pct*60/100) MOVED from scanner.rs into Rssi::from_nmcli_percent (single source of truth); scanner.rs now calls Rssi::from_nmcli_percent(pct).dbm(). Pinned by rssi.rs test (100->-30, 0->-90, 50->-60).

Provenance threaded in-memory scanner->trilateration (NO wire/DB change, all values stay i32): ScannedNetwork.rssi_source (parsers stamp iw=IwDbm / nmcli=NmcliPercentLinearized) -> ScanEntry -> Candidate -> PositionedAp.rssi_source. trilaterate() READS it via a debug! source-mix log (observability only; avoids dead_code; no weighting change).

vote_weight refactored to fn vote_weight(Option<Rssi>, f64)->f64; math unchanged (10^(clamp(dbm)/20)). Confirmed NUMERICALLY IDENTICAL: all pre-existing trilateration numeric tests (test_two_aps_equal_weight, test_stronger_signal_more_weight, antipodal_equal_weight_triggers_fallback, missing_signal_never_outweighs_strongest_measured, extreme_signal_dbm_yields_finite_position, antimeridian/polar) pass UNCHANGED.

Gotcha: large-but-mechanical churn (81 PositionedAp literals in trilaterate.rs, 17 ScanEntry, proptests + fuzz_trilaterate) — done via scoped scripts inserting rssi_source: IwDbm, then cargo fmt. Synthetic test dBm are raw-dBm-like, so IwDbm is the honest label.

Gate: nix develop -c just e2e => PASS (exit 0). whereamid: lib 226 passed, bin 490 passed, proptests 12, shutdown 4; 0 failed across workspace. cargo fmt --check => clean (exit 0).

## Review gate (orchestrator, independent) — GO/GO, no hardening needed
- QA (2x): 849/849, zero flake; clippy 0 -D warnings; fmt clean. nmcli-linearization pins (100->-30,0->-90,50->-60) + unchanged trilateration numerics all pass.
- Architecture: all 5 items CONFIRMED. Weights BIT-IDENTICAL (vote_weight body unchanged, only param type Option<i32>->Option<Rssi>; no signal value or assertion edited; stronger_rssi test is net-new). Linearization MOVED not duplicated (only in Rssi::from_nmcli_percent). Churn honest (~80 test literals get rssi_source: IwDbm on raw-dBm values -> honest label). repr(transparent)+RssiSource-on-PositionedAp is the AC#2-sanctioned alternative.
- KEY QUESTION answered: provenance is GENUINELY preserved end-to-end, NOT hollow. Traced scanner stamps source -> ScanEntry -> debounce ring (stores whole ScanEntry, NO signal averaging -> no source-collapse) -> Candidate (server.rs:1738) -> PositionedAp (server.rs:1623). Signal+source BOTH come from the live-scan candidate; the cache caches POSITIONS only, never signal. nmcli is the PRIMARY backend (iw fallback), so NmcliPercentLinearized is the common production case, not dead. Observability debug! at trilaterate.rs:641 reads the real source.
- from_iw_dbm re-wrap at the two vote_weight call sites: BENIGN (vote_weight is source-agnostic; reads only rssi.dbm(); the real provenance on ap.rssi_source is untouched) — already documented in-code; a comment flags it as a guard point IF vote_weight ever becomes source-aware.
- Two LOW non-defects (no action): from_nmcli_percent extrapolates for percent>100/<0 (nmcli guarantees 0-100; pure move, no regression; noted for 0271); the re-wrap semantic smell (inert, documented).
- Follow-up already filed by implementer: task-0271 (expose rssi provenance on the wire).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Introduced an Rssi(i32) newtype and RssiSource provenance so the two signal backends stop looking like the same physical quantity.

Context (stale-premise corrected): the weight fn already existed and was named (vote_weight), and nmcli percent was already converted to approximate dBm at the scanner — so there was NO raw-percent-as-dBm bug. This task is TYPE-SAFETY + PROVENANCE, not a numeric fix.

What changed:
- New whereamid/src/rssi.rs: #[repr(transparent)] Rssi(i32) with from_iw_dbm (identity) and from_nmcli_percent (the -90 + pct*60/100 linearization, MOVED here from the scanner so the mapping lives in one place), plus dbm(). enum RssiSource { IwDbm, NmcliPercentLinearized }.
- vote_weight refactored to take Option<Rssi> (math unchanged: 10^(clamp(dbm)/20)); weights are numerically identical.
- Provenance threaded daemon-internal: scanner stamps source -> ScannedNetwork/ScanEntry -> Candidate -> PositionedAp.rssi_source. Read for an observability debug! source-mix log (also anchors the field as genuinely used). Type-level docs: nmcli is a coarse biased approximation, iw authoritative, trilateration MAY down-weight/refuse to mix in future (not done here).

Why: the bare signal_dbm: i32 erased that nmcli values are a biased linearized approximation vs iw authoritative dBm; a future contributor could read false precision into the weight.

User impact: NONE — pure type refactor; identical positions/weights. Provenance is now tracked in-memory (nmcli is the primary backend, so it is genuinely exercised).

Scope: daemon-internal only; wire (NetworkEntry/DebugBssid signal_dbm) + DB stay i32. Wire exposure of provenance deferred to follow-up task-0271.

Tests: 849 workspace (+8), two independent gate runs zero flake, clippy 0, fmt clean. New: stronger_rssi_has_greater_vote_weight, from_iw_dbm identity, nmcli linearization pins (100/0/50 -> -30/-90/-60); all pre-existing trilateration numerics unchanged.

Design note: repr(transparent) forbids a second field, so RssiSource rides on PositionedAp (the AC#2-sanctioned alternative), not inside Rssi.

Commit: 1d8decd.
<!-- SECTION:FINAL_SUMMARY:END -->
