---
id: TASK-0123
title: 'Channel must be band-aware: TwoFourGhz/FiveGhz/SixGhz enum'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:18'
updated_date: '2026-07-09 02:35'
labels:
  - correctness
  - domain
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #2. Wi-Fi 6E/7 added 6 GHz operation where channel numbers restart at 1, overlapping 2.4 GHz. Channel 1@2412MHz and Channel 1@5955MHz are DIFFERENT physical channels. (channel, band) is the actual identifier. scanner.rs:17 stores channel as Option<i32>; db.rs:17 + SQL 'aps.channel INTEGER' has no band. freq_to_channel (scanner.rs:302) returns the IEEE-correct number per band but the type immediately erases the band. After parsing, channel=Some(1) could mean 2.4 GHz ch1 OR 6 GHz ch1.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Replace channel: Option<i32> with channel: Option<Channel> where enum Channel { TwoFourGhz(u8), FiveGhz(u16), SixGhz(u16) }
- [x] #2 Channel implements Display showing band + number (e.g. '6G/37')
- [x] #3 freq_to_channel returns Option<Channel> with the right variant per frequency range
- [x] #4 Schema migration v6: aps.channel column either dropped (if frequency is the canonical source) OR widened to TEXT carrying the band-prefixed string
- [x] #5 Unit tests: freq_to_channel(2412) -> TwoFourGhz(1); freq_to_channel(5955) -> SixGhz(1); freq_to_channel(5180) -> FiveGhz(36)
- [x] #6 Database tests verify round-trip (insert Channel, read back same variant)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add Channel enum {TwoFourGhz(u8),FiveGhz(u16),SixGhz(u16)} to whereami-protocol: Display=band-prefixed (2G/1,5G/36,6G/37), FromStr mirror, Serialize=string, Deserialize=string; from_frequency(i32)->Option<Channel> carrying EXACT boundary logic (2.4 table, 5935 special, 6GHz-before-5GHz).
2. NetworkEntry.channel: Option<Channel> with tolerant deserializer (null->None, string->parse, legacy int->None) + serde(default).
3. Scanner: ScannedNetwork.channel Option<Channel>, derived from frequency (2.4 fallback when freq absent); replace freq_to_channel with Channel::from_frequency; port boundary tests.
4. debounce ScanEntry.channel Option<Channel>.
5. DB: DROP aps.channel + pending.channel at v9 (SCHEMA_VERSION 8->9, migrate_v8_to_v9 guarded DROP COLUMN in run_migration). Keep channel in create_schema_v1 (authentic v1) - deviates from literal omit-from-CREATE to preserve historical-shape authenticity + exercise real DROP. Drop ApInfo.channel field; fix all SELECT/INSERT/row-mapping + all ApInfo literals.
6. Client render: print Channel Display (drop ch prefix).
7. Tests: from_frequency variants, DB freq->Channel round-trip, wire Channel round-trip, SCAN_FIXTURE update, extend build_historical_shape+cascade to v8, open_v8_shape_drops_channel_column_at_v9.
8. nix develop -c just e2e + cargo fmt --check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented (commit d014c8a), left In Progress for orchestrator review gate.

- Added whereami_protocol::Channel {TwoFourGhz(u8),FiveGhz(u16),SixGhz(u16)}: Display/FromStr band-prefixed (2G/6,5G/36,6G/37), serde as string, from_frequency carries EXACT boundary logic (6GHz-before-5GHz, 5935 special).
- Scanner ScannedNetwork.channel + debounce ScanEntry.channel + wire NetworkEntry.channel now Option<Channel>, derived from frequency (canonical). Bare nmcli/iw channel used only as 2.4GHz fallback when freq absent.
- Schema v8->v9 (migrate_v8_to_v9): ALTER TABLE aps/pending DROP COLUMN channel, column_exists-guarded + version-stamped in one run_migration tx. DROP COLUMN confirmed working on bundled SQLite by open_v8_shape_drops_channel_column_at_v9 test.
- ApInfo.channel field dropped; get_ap/upsert/lookup/pending SQL + all literals updated.
- Legacy-int wire deserialize: tolerant field deserializer maps a legacy bare integer to None (documented; band-less number cannot reconstruct a Channel) rather than erroring.

GATE: nix develop -c just e2e PASS; cargo fmt --check PASS; clippy -D warnings PASS; 841 workspace tests pass.

DEVIATION (documented): kept channel in create_schema_v1 (authentic v1) rather than omitting from CREATE TABLE, so historical-shape builder stays authentic and the v8->v9 drop test exercises a real column. Fresh DB creates-then-drops within one open() cascade; end state channel-less either way.

LIMITATION: docs/dataflow-persist.drawio still shows schema version=7 (pre-existing staleness, not touched).

## Review gate (orchestrator, independent) — GO/GO, no hardening needed
- QA (2x): 841/841, zero flake; clippy 0 -D warnings; fmt clean. All 6 named tests green (from_frequency_band_boundaries, aps_frequency_roundtrip_derives_band_qualified_channel, open_v8_shape_drops_channel_column_at_v9, cascade v1..=8, lockstep v8->v9, network_entry_tolerates_legacy_integer_channel).
- Architecture: all 6 items CONFIRMED. Band-boundary logic preserved (6GHz-before-5GHz + 5935 special case). Migration lossless + ATOMIC (drop {aps,pending}.channel PRAGMA-gated + version stamp in one run_migration tx; idempotent; frequency survives). create_schema_v1 deviation is SOUND ENGINEERING (v1 is the frozen historical DDL build_historical_shape replays; editing it would fabricate a never-existed shape + no-op the drop test) — correct, not a shortcut. NO stale aps.channel read: all 3 production SELECTs dropped channel and shifted row.get indices consistently (source 9->8, frequency 6->5, city 7->6, country 8->7, is_mobile 10->9) — NO off-by-one. Legacy-int wire deserialize is field-level tolerant (int->None, malformed string still errors). pending.channel drop test-only. Wire SCAN_FIXTURE + client render updated; protocol.md documents int->string.
- Two LOW non-defects (no action): (1) my brief overstated "reorder misclassifies" — the 5/6GHz ranges are disjoint today so ordering is future-proofing (code comment already says so); (2) legacy bare-int on wire -> None vs local scanner 1..=14->2G heuristic — defensible (cross-version wire is display-only + untrusted), documented.
- Filed follow-up: drawio schema-version label stale (7 -> 9).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Made (channel, band) a first-class typed identifier, fixing Wi-Fi 6E/7 channel-number ambiguity (6GHz ch1 != 2.4GHz ch1), and dropped the redundant persisted channel column (frequency is canonical).

What changed:
- New whereami_protocol::Channel { TwoFourGhz(u8), FiveGhz(u16), SixGhz(u16) } with Display/FromStr as a band-prefixed string ("2G/6","5G/36","6G/37") and Channel::from_frequency carrying the exact band-boundary logic (2.4GHz discrete table, 6GHz 5935 special case + 5955..=7115 range checked BEFORE the 5GHz 5180..=5885 range).
- channel: Option<i32> -> Option<Channel> across scanner (ScannedNetwork/ScanEntry), wire (NetworkEntry), and the daemon internals that threaded it (debounce/pending/server).
- Schema migration v8->v9 (SCHEMA_VERSION 8->9): drops the redundant aps.channel + pending.channel columns (frequency is the canonical physical source; channel is a pure function of it) in one atomic run_migration transaction, PRAGMA-gated + idempotent. ApInfo.channel removed; Channel derived from frequency on read.
- Wire: NetworkEntry.channel serializes as the band-prefixed string; a field-level tolerant deserializer maps an old daemon\x27s legacy bare int to None (a band-less number cannot reconstruct the (band,channel) identity). Client scan render prints Channel::Display.

Why: a bare channel number is ambiguous across bands post-6E; the physical identity is (channel, band), fully derivable from frequency — so the stored channel column was redundant state (SSOT).

User impact: scan output now shows band-qualified channels (e.g. 6G/37); cross-version scan display of a legacy int shows no channel (display-only, documented). No effect on positioning.

Tests: 841 workspace (+7), two independent gate runs zero flake, clippy 0, fmt clean. New: from_frequency band-boundary (ported, band-carrying), DB frequency->derived-Channel round-trip, v8->v9 drop-column + cascade v1..=9, wire per-band round-trip + legacy-int tolerance.

Design note: create_schema_v1 intentionally retains channel (the frozen historical DDL that build_historical_shape replays); fresh DBs create-then-drop through the cascade. Follow-up filed for a stale drawio schema-version label.

Commit: d014c8a.
<!-- SECTION:FINAL_SUMMARY:END -->
