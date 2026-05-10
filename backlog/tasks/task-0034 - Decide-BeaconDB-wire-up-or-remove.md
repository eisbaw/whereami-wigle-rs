---
id: TASK-0034
title: 'Decide BeaconDB: wire up or remove'
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 05:37'
updated_date: '2026-05-10 07:28'
labels:
  - decision
  - cleanup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
BeaconDbClient is constructed in main.rs and stored on DaemonState but never called. After task-0027 it is annotated #[allow(dead_code)] with a 'kept for future' comment, but that future has been pending since the original design. Either integrate it into resolve_chain as a third Provider variant (Apple → WiGLE → BeaconDB), or delete the module + field + construction entirely. Pick one.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Either: BeaconDB is wired as a Provider variant and exercised by an integration test, OR the module + DaemonState.beacondb + main.rs construction are all deleted
- [x] #2 No #[allow(dead_code)] remaining for beacondb-related code
- [x] #3 PRD updated to match the chosen direction
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Decision: REMOVE BeaconDbClient. Reason: BeaconDB API returns aggregate position not per-AP; incompatible with Provider trait per-BSSID architecture. Wiring would require either a different abstraction or faking per-AP positions (which loses information). Better to remove than carry dead code.
2. Keep Source::BeaconDb enum variant in db.rs for read-back of historical DB rows (priority 20).
3. Delete whereamid/src/beacondb.rs
4. Remove mod beacondb, DaemonState.beacondb, construction in main.rs
5. Remove BeaconDbConfig and default_true from config.rs
6. Update PRD if it mentions BeaconDB as a runtime backend
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Decision: REMOVE. The BeaconDB API returns aggregate position not per-AP, which is incompatible with the per-BSSID Provider chain architecture from task-0023. Wiring it would require a different abstraction (multi-BSSID-at-once provider) or faking per-AP positions (information loss). The cost of carrying it dead — Arc field, HTTP client at startup, dead-code annotation, cognitive overhead — exceeded the option value of keeping it.

Removed: whereamid/src/beacondb.rs (the whole module), mod beacondb declaration in main.rs, BeaconDbClient construction, DaemonState.beacondb field, BeaconDbConfig + default_true in config.rs, BeaconDbClient import in server.rs, BeaconDB mention in http.rs comment.

Kept: Source::BeaconDb enum variant in db.rs (priority 20, mapped from 'beacondb' string) so historical DB rows from earlier prototypes still read back at the correct priority. The Source enum is the wire-format-stable boundary; deleting the variant would silently demote those rows to Unknown (priority 0).

PRD updated: removed BeaconDB from the architecture diagram, added a note explaining the Source enum still recognises beacondb for backwards compat with old DBs but no client runs today.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Removed the dead BeaconDbClient module and all its wiring (DaemonState field, main.rs construction, BeaconDbConfig). Kept Source::BeaconDb enum variant (priority 20) so historical DB rows still read back correctly. PRD updated to reflect Apple+WiGLE as the only runtime backends. Build/test/clippy/fmt all clean. No #[allow(dead_code)] remains for beacondb-related code.
<!-- SECTION:FINAL_SUMMARY:END -->
