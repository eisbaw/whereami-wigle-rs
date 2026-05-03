---
id: TASK-3
title: CLI arg parsing + TOML config loading
status: Done
assignee: []
created_date: '2026-05-02 00:05'
updated_date: '2026-05-02 00:13'
labels: []
dependencies:
  - TASK-1
references:
  - PRD.md
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement config.rs using clap for CLI args and toml for secrets file. All operational params as CLI args with defaults: --bind, --db, --interface, --scan-interval-fast, --scan-fast-duration, --scan-interval-slow, --debounce-window, --debounce-threshold, --top-n, --pending-interval, --pending-max-attempts, --daily-limit, --config. TOML file for wigle api_user/api_key and beacondb.enabled.
<!-- SECTION:DESCRIPTION:END -->
