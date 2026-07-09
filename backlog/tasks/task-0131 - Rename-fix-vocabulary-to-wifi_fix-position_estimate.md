---
id: TASK-0131
title: Rename fix vocabulary to wifi_fix / position_estimate
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:20'
updated_date: '2026-07-09 01:21'
labels:
  - docs
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #10. 'Fix' in geolocation almost always means GPS fix (satellite-derived position). This daemon never sees GPS — every 'fix' is a Wi-Fi centroid estimate. db.rs:75 FixRow, db.rs:88 LastFixRow, server.rs LastFix, schema 'fixes' table. A reader from GPS background will assume accuracy_m has GPS semantics (HDOP-derived radius) — see task-0127 for why it doesn't.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Doc-comments on FixRow, LastFixRow, LastFix explicitly say: 'a Wi-Fi-trilateration-derived position estimate, NOT a GPS fix'
- [ ] #2 Optional (heavier): rename to WifiFixRow, LastWifiFix, schema table 'wifi_fixes' via migration v6 (CLI flags --wifi-history-* etc.) — this is the cheapest of the Compass findings to JUST DOC but the heaviest to fully rename
- [x] #3 Decision recorded in task notes: doc-only OR full rename
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DECISION: DOC-ONLY (AC#3). Added the Wi-Fi-trilateration-vs-GPS terminology note to the two current domain structs.

Note on AC#1 naming drift: the AC lists FixRow/LastFixRow/LastFix, but task-0134 already collapsed those into the single domain types db::Fix (db.rs:1643) and db::LastFix (db.rs:1748) — there is no longer a separate LastFixRow or server::LastFix. Documented both current structs; the phrase "a Wi-Fi-trilateration-derived position estimate, NOT a GPS fix" is on each, plus the accuracy_m-is-not-HDOP nuance (ties to task-0127).

AC#2 (full rename to WifiFix/wifi_fixes table + --wifi-history-* CLI via a new migration) NOT taken: it is the heaviest of the Compass findings and doc-only captures the actual clarity value (disambiguating GPS vs Wi-Fi semantics) without a schema migration + CLI-flag break. If a full rename is later wanted, file it as a fresh task (schema migration vN + flag aliases, same pattern as task-0130).

Verification: doc-only change; nix cargo fmt --check clean, clippy -D warnings (incl rustdoc intra-doc link lints) clean. Done in-thread (trivial doc change; not delegated).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Documented that db::Fix and db::LastFix represent Wi-Fi-trilateration position estimates, not GPS fixes (Compass-domain finding #10).

Doc-only decision (AC#3): added an explicit TERMINOLOGY note to both domain structs stating the position is Wi-Fi-derived (the daemon has no GNSS input) and that accuracy_m is a trilateration confidence radius, not a GPS HDOP value (task-0127). The heavier full rename (WifiFix types, wifi_fixes table via migration, --wifi-history-* flags) was explicitly not taken — doc-only delivers the disambiguation value without a schema/CLI break.

Note: the AC named FixRow/LastFixRow, but task-0134 had already merged those into the single db::Fix / db::LastFix domain types; both current structs are documented.

Verification: fmt clean, clippy -D warnings clean (incl intra-doc link lints). No behavioral change.

Commit: (see git log for the docs(db) commit).
<!-- SECTION:FINAL_SUMMARY:END -->
