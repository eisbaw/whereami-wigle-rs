---
id: TASK-0096
title: 'Daemon-side periodic locate: write fixes timeseries autonomously'
status: To Do
assignee: []
created_date: '2026-05-19 06:53'
updated_date: '2026-05-19 07:08'
labels:
  - observability
  - history
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Today, the fixes table is populated ONLY when a client calls /locate. A user who runs 'whereami locate' once in the morning then moves around without re-querying leaves an empty history. Real incident: user has only 25 fixes over 7 days, all from manual locate invocations; most of the actual trajectory is missing.

Add a background autolocate loop in the daemon, parallel to the existing scan / pending-drain / history-prune loops (task-0075 cooperative-shutdown pattern). Default ENABLED at 120s interval (similar to last_fix already updated on every /locate; this is incrementally similar in privacy terms). 0 disables.

Edge cases:
- First fix ever (last_fix is NULL): always write — dedup check is skipped.
- Empty scan (no stable BSSIDs visible): skip; do not write a useless fix.
- No cached APs to trilaterate from: skip.
- Dedup: skip when last_fix is non-NULL AND haversine_m(new, last_fix) < history_segment_distance_m AND (now - last_fix.at) < autolocate_interval_secs.

This composes with task-0094 (the implied-speed and confidence fields populate every autolocate fix too, so the trajectory carries quality metadata) and task-0095 (raw history view becomes useful with denser fixes).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add --autolocate-interval-secs <N> CLI flag (default 120; 0 disables); also expose as services.whereami.autolocateIntervalSecs in nix/module.nix and homeManagerModules
- [ ] #2 Add run_autolocate_loop background task in main.rs alongside the scan/drain/prune loops; uses tokio::select! on DaemonState.shutdown like task-0075; max graceful-shutdown wait of 5s matches task-0075's existing budget
- [ ] #3 Loop body extracts the locate logic into a shared internal function compute_locate(state) -> Result<LocateOutcome> called by both handle_locate and run_autolocate_loop; the autolocate path does NOT touch any TCP connection
- [ ] #4 Skip-conditions checked in order: (a) no stable BSSIDs in debouncer -> skip; (b) no cached APs after debouncer filter -> skip; (c) last_fix is non-NULL AND haversine_m(new_pos, last_fix) < history_segment_distance_m AND age(last_fix) < autolocate_interval_secs -> skip-as-dedup. First-fix (last_fix NULL) bypasses dedup and writes unconditionally
- [ ] #5 Add Stats counter autolocate_fixes_written: u64; wired into DaemonState.stats following the pattern of inflight and db_write_failures (task-0074/0076); exposed via /stats response and whereami stats CLI line
- [ ] #6 Test (integration-style in whereamid/tests/): build a DaemonState with a test scanner trait impl that returns a fixed Vec<ScannedAp> with cached positions in aps table; set autolocate_interval_secs=1; spawn run_autolocate_loop; wait 3s; assert fixes table has >= 2 rows and autolocate_fixes_written counter is >= 2. Use tokio::time::pause + advance if the scanner is exercised purely in-process; otherwise sleep 3s real time
- [ ] #7 Test: dedup — run autolocate twice with identical scanner output and no movement; assert second invocation does NOT write a new fix (counter unchanged, fixes table size unchanged)
- [ ] #8 Test: first-fix — autolocate on empty fixes table writes the row even though it is the very first one
- [ ] #9 README updated: brief paragraph in 'Caching and offline operation' or 'Architecture' covering autolocate; trade-off (richer history vs more disk writes) noted; --autolocate-interval-secs=0 documented as the off-switch
<!-- AC:END -->
