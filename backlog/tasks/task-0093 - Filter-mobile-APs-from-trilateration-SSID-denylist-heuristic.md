---
id: TASK-0093
title: Filter mobile APs from trilateration (SSID denylist + heuristic)
status: To Do
assignee: []
created_date: '2026-05-19 06:48'
updated_date: '2026-05-19 07:04'
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
- [ ] #1 Add --mobile-ssid-denylist CLI flag accepting comma-separated glob patterns (POSIX fnmatch-style, case-insensitive); default value as specified in description; flag is also exposed in nix/module.nix as services.whereami.mobileSsidDenylist
- [ ] #2 Schema migration v5 -> v6 adds aps.is_mobile INTEGER NOT NULL DEFAULT 0 column; migration is transactional (uses run_migration helper from task-0047)
- [ ] #3 upsert_ap recomputes is_mobile on every call: 1 if AP's SSID is non-NULL, non-empty, non-'<hidden>', AND matches any denylist pattern; else 0. Uses the same fnmatch implementation everywhere (single helper fn in scanner.rs or new mobility.rs)
- [ ] #4 filter_outliers and trilaterate exclude APs with is_mobile=1; this happens BEFORE drop_isolated so a cluster of train APs does not save itself
- [ ] #5 Existing rows are NOT retroactively re-evaluated when the denylist changes; the doc-comment on is_mobile says so explicitly
- [ ] #6 Unit test: 3 fixed APs + 2 mobile-flagged APs -> trilaterate returns the fixed-AP centroid; filter_outliers returns only the fixed APs
- [ ] #7 Unit test: NULL ssid AP with B8:50:01 OUI is NOT flagged (hidden home routers can share OUI with trains)
- [ ] #8 Unit test: glob 'MOVIA*' matches 'MOVIA_BUS_42' but not 'mymoviabus' (anchored at start; case-insensitive)
- [ ] #9 README updated: brief 'Mobile-AP filtering' paragraph in the Architecture section noting this is a coarse SSID-based filter; the principled mechanism is task-0094
<!-- AC:END -->
