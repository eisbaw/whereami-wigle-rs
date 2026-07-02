---
id: TASK-0197
title: >-
  State dir and SQLite DB are world-readable (system module + hm-module + manual
  runs)
status: To Do
assignee: []
created_date: '2026-07-02 04:52'
labels:
  - security
  - privacy
  - nix
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
nix/module.nix sets StateDirectory=whereami with no StateDirectoryMode (systemd default 0755) and no UMask (default 0022 -> aps.sqlite 0644). The daemon never chmods. The system module is incidentally protected only because DynamicUser puts state under /var/lib/private (0700); the hm-module (db under ~/.local/share) and any --db-relocated/manual run leave aps.sqlite - which holds the fixes location-history table, last_fix, and every BSSID/SSID seen - readable by other local users, even when the daemon is stopped. Ref nix/module.nix:22-23, whereamid/src/main.rs:57-61, db.rs:104-105.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 on a fresh start the DB file and its directory are not group/other readable (verified by mode bits) on system, hm, and manual paths
- [ ] #2 set StateDirectoryMode=0700 and UMask=0077 in both nix units; daemon creates the state dir 0700 and DB 0600
- [ ] #3 existing databases are tightened (or a warning emitted) on startup
<!-- AC:END -->
