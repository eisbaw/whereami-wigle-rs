---
id: TASK-0269
title: >-
  Propagate Bssid newtype across DB storage rows and whereami-protocol wire
  types
status: To Do
assignee: []
created_date: '2026-07-09 01:46'
labels:
  - correctness
  - domain
  - refactor
dependencies:
  - TASK-0122
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up to task-0122, which introduced whereamid/src/bssid.rs (Bssid([u8;6])) and parses String->Bssid at the system boundaries (scanner parse sites, apple decode_response, server handle_resolve, the trilateration filter in solve_locate_from_cache, and the WiGLE query) but deliberately left storage/wire types as String to keep that diff reviewable. This task completes the adoption: replace String bssid fields with Bssid in db::ApInfo and the aps rows, and in the whereami-protocol wire types (ResolveResultEntry.bssid, ScanResponse/NetworkEntry.bssid, DebugBssid), parsing once at deserialization instead of at each use. Removes the repeated .parse::<Bssid>() calls at boundaries.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 db::ApInfo.bssid and the aps table row use Bssid (serialized as canonical string); upsert/lookup round-trip
- [ ] #2 whereami-protocol wire types (ResolveResultEntry, NetworkEntry/ScanResponse, DebugBssid) use Bssid via serde canonical-string (de)serialization
- [ ] #3 Boundary .parse::<Bssid>() calls added in task-0122 are removed where the type now guarantees validity
- [ ] #4 No behavioral change to trilateration eligibility or the WiGLE injection guard; existing tests still pass
<!-- AC:END -->
