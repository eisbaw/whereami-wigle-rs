---
id: TASK-0093
title: Filter mobile APs from trilateration (SSID denylist + heuristic)
status: Done
assignee:
  - task0093-implementer
created_date: '2026-05-19 06:48'
updated_date: '2026-07-05 12:37'
labels:
  - correctness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Mobile APs (trains, buses, taxis, e-scooters, personal hotspots) systematically poison trilateration because cached positions are wherever Apple/WiGLE last logged each BSSID, while the daemon assumes geographic fixity. Real incident: at Copenhagen Central Station, 8 DSB train Wi-Fi BSSIDs visible whose cached positions cluster around Maglevænget (7 km north) where the trains were last catalogued. This is the cheap fallback. The principled solution is task-0094.

Pattern syntax: glob (POSIX fnmatch-style: '*' matches any substring, '?' matches single char; no regex, no extended globs). Matching is case-insensitive against the AP's SSID field exactly as it came from the scanner. APs whose SSID is NULL or '<hidden>' are NOT auto-flagged — they may have valid hidden-network home APs. Heuristic mobility detection (flagging based on cached-position drift across successive WiGLE queries) is out of scope; if useful it gets a follow-up task.

Default denylist (literal SSIDs unless otherwise noted, comma-separated):
  DSB_Wi-Fi
  .DSB_Wi-Fi
  dsb-mng-wifi
  MOVIA*
  Bus_WiFi
  Bus WiFi
  *_iPhone
  *_Hotspot
  iPhone
  AndroidAP*

is_mobile is recomputed against the (current) denylist on every upsert_ap call, so updating the denylist takes effect for any AP the daemon next learns about. Existing rows are NOT retroactively re-evaluated — flag accept that as a known limitation in the doc-comment.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Add --mobile-ssid-denylist CLI flag accepting comma-separated glob patterns (POSIX fnmatch-style, case-insensitive); default value as specified in description; flag is also exposed in nix/module.nix as services.whereami.mobileSsidDenylist
- [x] #2 Schema migration v5 -> v6 adds aps.is_mobile INTEGER NOT NULL DEFAULT 0 column; migration is transactional (uses run_migration helper from task-0047)
- [x] #3 upsert_ap recomputes is_mobile on every call: 1 if AP's SSID is non-NULL, non-empty, non-'<hidden>', AND matches any denylist pattern; else 0. Uses the same fnmatch implementation everywhere (single helper fn in scanner.rs or new mobility.rs)
- [x] #4 filter_outliers and trilaterate exclude APs with is_mobile=1; this happens BEFORE drop_isolated so a cluster of train APs does not save itself
- [x] #5 Existing rows are NOT retroactively re-evaluated when the denylist changes; the doc-comment on is_mobile says so explicitly
- [x] #6 Unit test: 3 fixed APs + 2 mobile-flagged APs -> trilaterate returns the fixed-AP centroid; filter_outliers returns only the fixed APs
- [x] #7 Unit test: NULL ssid AP with B8:50:01 OUI is NOT flagged (hidden home routers can share OUI with trains)
- [x] #8 Unit test: glob 'MOVIA*' matches 'MOVIA_BUS_42' but not 'mymoviabus' (anchored at start; case-insensitive)
- [x] #9 README updated: brief 'Mobile-AP filtering' paragraph in the Architecture section noting this is a coarse SSID-based filter; the principled mechanism is task-0094
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Trace the current Args/config/Nix option flow, aps schema and upsert call sites, and PositionedAp construction so the denylist has one authoritative classifier and no split behavior.
2. Add a transactional v5->v6 migration for aps.is_mobile, persist a recomputed flag on every upsert, and document that existing rows change only when relearned.
3. Exclude mobile rows before outlier filtering/trilateration and add focused migration, matching, persistence, and algorithm regressions including hidden/NULL SSIDs.
4. Expose the comma-separated glob denylist through the daemon CLI and NixOS module, document the coarse architecture behavior, and verify config/module integration.
5. Run focused/full tests and formatting/lint, freeze the diff, pass mandatory parallel QA and architecture reviews, run final just e2e, close the tracker accurately, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Selected next by the Phase 3 orchestrator on 2026-07-05: unblocked correctness task, ahead of remaining low-priority documentation/tooling work. Sole code writer is task0093-implementer; the orchestrator owns tracker transitions and the mandatory review/commit gate.

Onboarding finding: scanner-observed SSID is currently lost at the resolver boundary. Apple is first in the background chain and returns ssid=None, so classifying provider ApInfo alone would make the feature ineffective. The implementation must snapshot scanner SSID before awaits/spawn, carry it through write-through resolution and pending retries, and persist that observed value. Position/source priority remains independently gated; ssid plus recomputed is_mobile update together on every upsert. v6 shape repair must also detect a missing aps.is_mobile column. All-mobile batches must fail rather than be resurrected by filter_outliers empty fallback.

First frozen review gate (hash 41b1e927996f97aead58216e9f08e75f9ba03438e36673f8f384fbba2a6d8cf0): QA and architecture NO-GO. Blocker 1: stable/fallback candidates were truncated to top_n before mobile filtering, so stronger mobile rows could starve weaker fixed rows; QA reproduced 11 MOVIA + 3 fixed yielding no fix. Blocker 2: Nix emitted a shell-escaped separate value, so leading -- patterns were parsed as options and systemd $/% expansion remained unsafe. Fix pass requires scanner-known filtering before top_n, persisted defense in depth, >top_n/all-mobile regressions, and one canonical systemd-escaped --mobile-ssid-denylist=<value> argv with adversarial encoding tests. The first gate otherwise passed 550 tests, strict clippy, migration/pending/dataflow checks, and packaged flag parity.

Second frozen re-review (hash df07937cdc6fc6621ead99358e1962e8fd88c5bcfc436db2c64275f01274f032): QA and architecture NO-GO on a persisted-classification sibling. Scanner-known mobiles were correctly filtered before top_n and the Nix/systemd repair was sound, but stronger rows with persisted is_mobile=1 and current hidden/nonmatching SSIDs could still occupy all pre-DB top_n slots, then be removed by defense in depth while weaker fixed rows were never queried. Root fix: apply top_n after scanner, persisted-mobile, and finite-position eligibility while keeping fallback provider work bounded; add stable and fallback mismatch regressions. Also correct the all-mobile visible-scan error from no APs detected yet to no eligible fixed APs detected. Re-review artifact checks hit ENOSPC; the orchestrator safely removed only this repository’s generated Cargo targets (8.6 GiB), restoring 15 GiB available.

Final frozen gate (hash 2c0e6bdf3177feac7bd64a917f9c438922f4a1da77b2d88c32a71a79a91441cd): QA and architecture GO. The final selection order is scanner denylist, ordered cache lookup, persisted-mobile/finite eligibility, then top_n; stable and fallback regressions cover stronger stale-mobile and non-finite rows ahead of weaker fixed rows. Cold-start provider refill excludes cached rows before a hard top_n bound. NixOS/HM emit one systemd-escaped equals-form argument with adversarial encoding, CR/LF rejection, and Rust/Nix default parity. Final reviewer evidence: 23 focused QA tests, 557-test e2e plus strict clippy, packaged flag-parity/option-bounds, formatting, diff hygiene, and frozen hash all passed. The exact docs/protocol.md source-filter addition is an integration prerequisite for the packaged contract test and is narrowly scoped.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented coarse mobile-AP filtering end to end. Scanner-observed SSIDs now survive Apple/WiGLE resolution and pending retries; schema v6 persists a writer-recomputed is_mobile flag while preserving position-source priority. Current scanner matches, persisted mobile rows, and non-finite positions are excluded before top-N/outlier/trilateration, with bounded fallback resolution and accurate source accounting. Added exact glob/default/hidden/migration/rollback/shape-repair/dataflow/starvation/all-mobile tests, shared NixOS/Home Manager configuration with systemd-safe argv encoding, and README/ADR/protocol documentation. Existing cache flags intentionally change only on a later successful upsert; current scanner denylist matches still take effect immediately for positioning. Mandatory QA and architecture reviews passed after two honest NO-GO fix cycles.
<!-- SECTION:FINAL_SUMMARY:END -->
