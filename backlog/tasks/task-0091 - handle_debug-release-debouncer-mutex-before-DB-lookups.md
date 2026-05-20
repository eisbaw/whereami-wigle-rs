---
id: TASK-0091
title: 'handle_debug: release debouncer mutex before DB lookups'
status: To Do
assignee: []
created_date: '2026-05-13 21:55'
labels:
  - robustness
  - server
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
server.rs::handle_debug acquires the (tokio) debouncer mutex at the top, then iterates over all_bssids doing three sequential SQL queries per BSSID (get_ap, is_not_found, is_pending), each acquiring the std DB mutex. With a busy debouncer (20+ visible BSSIDs) that is 60+ index probes, all while blocking handle_locate and handle_scan from acquiring the debouncer. 'watch -n1 whereami debug' on a multi-AP environment will interfere with locate latency. Fix: snapshot the data needed out of the debouncer into a local Vec, release the guard, then run DB lookups outside the lock. Or batch the DB queries into one statement (SELECT bssid,... FROM aps WHERE bssid IN (?,?,?)) so the std DB lock is held once. The snapshot-then-release pattern is simpler and matches what handle_stats already does.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 handle_debug acquires the debouncer mutex, snapshots the BSSID list + counters into a local Vec, then drops the guard before any DB calls
- [ ] #2 DB lookups happen with the debouncer NOT held
- [ ] #3 Stretch (optional): batch get_ap / is_not_found / is_pending into single IN-clause queries so the DB mutex is taken once per query type, not once per BSSID
- [ ] #4 Test: concurrent handle_debug + handle_locate (e.g. spawn both, race them, assert neither blocks the other beyond a small bound)
<!-- AC:END -->
