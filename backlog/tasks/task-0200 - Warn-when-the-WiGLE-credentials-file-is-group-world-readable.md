---
id: TASK-0200
title: Warn when the WiGLE credentials file is group/world-readable
status: To Do
assignee: []
created_date: '2026-07-02 04:52'
labels:
  - security
  - privacy
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
load_config_file reads the config with no permission check, and the README secret-store section shows the [wigle] api_key file without any chmod 600 guidance. On a multi-user host another local user can read the WiGLE API key if the file was created with a default umask (0644). Ref whereamid/src/config.rs:192-206, README.md:51-59.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 starting the daemon with a world/group-readable config file emits a clear warning naming the file and the risk (mode & 0o077 != 0)
- [ ] #2 a 0600 config file produces no warning
- [ ] #3 docs instruct users to keep the file at 0600
<!-- AC:END -->
