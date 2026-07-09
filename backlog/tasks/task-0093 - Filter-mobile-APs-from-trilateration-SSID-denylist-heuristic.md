---
id: TASK-0093
title: Filter mobile APs from trilateration (SSID denylist + heuristic)
status: Done
assignee:
  - task0093-implementer
created_date: '2026-05-19 06:48'
updated_date: '2026-07-09 01:57'
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
Closed as SUBSUMED by task-0122 (BSSID U/L-bit newtype + locally-administered exclusion from trilateration), landed 2026-07-09 (commit 330ff8e). The randomized-MAC exclusion is the principled superset of an SSID denylist: a randomized MAC seen in different cities is different devices wearing the same address, and is now excluded at the trilateration boundary by Bssid::is_locally_administered(), regardless of SSID. No separate SSID denylist is needed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented coarse mobile-AP filtering end to end. Scanner-observed SSIDs now survive Apple/WiGLE resolution and pending retries; schema v6 persists a writer-recomputed is_mobile flag while preserving position-source priority. Current scanner matches, persisted mobile rows, and non-finite positions are excluded before top-N/outlier/trilateration, with bounded fallback resolution and accurate source accounting. Added exact glob/default/hidden/migration/rollback/shape-repair/dataflow/starvation/all-mobile tests, shared NixOS/Home Manager configuration with systemd-safe argv encoding, and README/ADR/protocol documentation. Existing cache flags intentionally change only on a later successful upsert; current scanner denylist matches still take effect immediately for positioning. Mandatory QA and architecture reviews passed after two honest NO-GO fix cycles.
<!-- SECTION:FINAL_SUMMARY:END -->
