---
id: TASK-0236
title: >-
  whereami -V/--version should print the client version locally, not query the
  daemon
status: To Do
assignee: []
created_date: '2026-07-03 07:33'
labels:
  - client
  - ux
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
QA finding (cycle 9, task-0209 gate, L5): the client's -V/--version flag is documented in --help as 'equivalent to the version subcommand', which CONNECTS TO THE DAEMON. Convention for --version is a local, offline answer (print client version + embedded GIT_REV and exit 0). Surprising UX: whereami --version fails with a transport error when no daemon is reachable. Keep the version SUBCOMMAND daemon-querying (it reports daemon version/git_rev); make the -V/--version FLAG local-only. Sites: whereami-client/src/main.rs argv handling + help text; add a test that -V succeeds with WHEREAMI_SOCKET pointed at an absent path.
<!-- SECTION:DESCRIPTION:END -->
