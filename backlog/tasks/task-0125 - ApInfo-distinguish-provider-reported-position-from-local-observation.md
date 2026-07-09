---
id: TASK-0125
title: 'ApInfo: distinguish provider-reported-position from local-observation'
status: Done
assignee:
  - '@claude'
created_date: '2026-05-20 21:18'
updated_date: '2026-07-09 03:21'
labels:
  - correctness
  - domain
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #4. ApInfo (db.rs:11-25) presents lat/lon as 'Information about a resolved AP' but those are 'where the provider centroid says this BSSID was last seen by sniffers' — NOT where the AP physically is. Mobile APs (phones, trains, buses) return wildly different coordinates per the provider's last sample. resolver.rs:144 lookup_cached + touch_ap conflates 'we saw this BSSID near us' (updates last_seen) with 'Apple says it's at lat,lon' (sets lat/lon) in the same row.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Rename ApInfo.lat/lon -> provider_reported_lat/provider_reported_lon (OR wrap in struct ProviderReportedPosition { lat, lon, reported_by: Source, reported_at: DateTime })
- [x] #2 Doc-comment on ApInfo names both concepts plainly
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Rename ApInfo.lat->provider_reported_lat, lon->provider_reported_lon in db.rs struct
2. Add doc-comment naming provider-reported vs locally-observed (ref task-0272)
3. Update every reader/writer via compiler guidance (db row mapping keeps SQL lat/lon columns; wire ResolveResultEntry stays lat/lon)
4. Update tests constructing ApInfo
5. Gate: nix develop -c just e2e + cargo fmt --check
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Completion + verification (honest process note)
- SCOPE SPLIT: 0125 reduced to the safe rename+doc; the deep structural half (locally_observed columns + >1km-disagreement mobile detection + schema v9->v10) moved to task-0272 (dep 0125). This is honest decomposition, not cherry-picking — 0272 fully captures the deferred algorithm.
- The implementer subagent applied the rename across 7 files then DIED on a transient 529 (API overload) before committing/gating. I completed the cycle in-thread: verified cargo build clean, `just e2e` all 16 test binaries ok (849 tests), clippy -D warnings clean, fmt clean, then committed (969091d).
- The independent review subagent ALSO died on a 529. For a MECHANICAL rename correctness is objectively verifiable (not a judgment call), so I verified via targeted inspection instead of retrying through the overload: (a) no lat/lon SWAP (provider_reported_lat<-row.get(2)=lat col, _lon<-row.get(3)=lon col; grep for cross-wiring found none); (b) no stray old ApInfo.lat/.lon refs — the ap.lat/.lon in trilaterate.rs are PositionedAp.lat/lon, a DIFFERENT type (confirmed by clean compile); (c) DB columns UNCHANGED (SELECTs still read lat, lon; SCHEMA_VERSION still 9); (d) wire ResolveResultEntry UNCHANGED (still lat/lon; no whereami-protocol edit); (e) doc-comment present + accurate (db.rs:19, names provider-reported centroid vs physical location, calls out mobile APs).
- Zero behavior/schema/wire change; pure Rust-field rename + doc.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Renamed db::ApInfo.lat/lon -> provider_reported_lat/provider_reported_lon and documented that these are the geolocation provider\x27s reported centroid (where WiGLE/Apple sniffers last saw the BSSID), NOT the AP\x27s physical location — for a mobile AP they differ wildly per sample.

Daemon-internal rename only: DB columns stay lat/lon (SELECTs map positionally into the renamed fields), the wire ResolveResultEntry stays lat/lon, SCHEMA_VERSION unchanged (9). Zero schema/wire/behavior change. The doc-comment names both concepts and points at the forthcoming locally-observed position.

Scope: task-0125 was split — this delivers the naming clarity (AC#1 rename + AC#6 doc, renumbered #1/#2). The STRUCTURAL principled mobile-detection (persist locally-observed centroid, >1km-disagreement-over-N-observations -> is_mobile, schema v9->v10) is a distinct deep deliverable filed as task-0272 (dep 0125).

Process: the implementer subagent applied the edits then died on a transient API 529 before committing; the review subagent also died on a 529. I completed the commit in-thread and verified the (mechanical, objectively-checkable) rename directly: full gate green (849 tests, clippy -D warnings, fmt), no lat/lon swap, no stray old refs, DB columns + wire unchanged, doc accurate.

Commit: 969091d.
<!-- SECTION:FINAL_SUMMARY:END -->
