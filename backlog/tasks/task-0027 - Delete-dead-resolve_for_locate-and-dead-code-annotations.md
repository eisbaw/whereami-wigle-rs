---
id: TASK-0027
title: Delete dead resolve_for_locate and dead code annotations
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-09 21:07'
updated_date: '2026-05-09 21:31'
labels: []
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
resolve_for_locate is dead (#[allow(dead_code)]). ResolveResult fields cached_count, fetched_count, pending_count are dead. BeaconDB is constructed but never called. Delete resolve_for_locate, clean up dead_code annotations, remove or properly annotate BeaconDB.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Delete resolve_for_locate (dead).
2. Drop unused fields (cached_count, fetched_count, pending_count) from ResolveResult; keep what is actually used.
3. Remove stale #[allow(dead_code)] on apple::AppleClient::lookup_bssid (it is called).
4. Remove module-level #[allow(dead_code)] on mod beacondb; keep narrow annotations only where needed.
5. cargo build / test / clippy -D warnings / fmt --check inside nix develop. Commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Deleted resolve_for_locate (dead).
Removed cached_count, fetched_count, pending_count fields from ResolveResult; only positioned + fetched_bssids remain.
Removed stale #[allow(dead_code)] on apple::AppleClient::lookup_bssid (it is called from resolver and pending).
Replaced module-level #[allow(dead_code)] on mod beacondb with an inner-attribute #\![allow(dead_code)] inside beacondb.rs and a comment explaining the whole module is reserved for future use.
Kept #[allow(dead_code)] on DaemonState.beacondb (field is genuinely unread).
build/test/clippy(-D warnings)/fmt all green.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Deleted dead resolve_for_locate path and stale dead-code annotations.

Changes:
- whereamid/src/resolver.rs: removed resolve_for_locate (never called) and the unused cached_count/fetched_count/pending_count fields on ResolveResult.
- whereamid/src/apple.rs: removed stale #[allow(dead_code)] on AppleClient::lookup_bssid (used from resolver and pending).
- whereamid/src/main.rs: removed broad #[allow(dead_code)] on `mod beacondb`; replaced with a documented inner-attribute #\![allow(dead_code)] inside beacondb.rs so the suppression lives next to the code it covers.
- DaemonState.beacondb retains its narrow #[allow(dead_code)] (the field is intentionally unread until BeaconDB is wired up).

BeaconDbClient itself was kept (deletion deferred per follow-up plan).

Verification: cargo build, cargo test (33 pass), cargo clippy --all-targets -- -D warnings, cargo fmt --check all clean.
<!-- SECTION:FINAL_SUMMARY:END -->
