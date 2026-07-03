---
id: TASK-0232
title: 'Client: first-class --socket/--addr CLI flags (currently env-var only)'
status: To Do
assignee: []
created_date: '2026-07-03 05:49'
labels:
  - client
  - cli
  - usability
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0196 made whereami-client resolve its target via WHEREAMI_SOCKET / WHEREAMI_ADDR env vars and default socket-path probing (whereami-client/src/lib.rs resolve_target + from_env; the CLI wires it in whereami-client/src/main.rs). There is intentionally no --socket/--addr CLI flag yet (the CLI is hand-rolled argv today; clap migration is task-0079's scope). Add first-class --socket <path> and --addr <host:port> global flags that override the env resolution, so users can point the CLI at a specific daemon without exporting env vars. Precedence: explicit flag > env > default probe.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 whereami --socket <path> connects to that unix socket, overriding env + default probing
- [ ] #2 whereami --addr <host:port> connects over TCP, overriding env + default probing
- [ ] #3 flag beats env beats default-probe; --json still emits a synthesized JSON line on resolution failure (task-0183 parity)
<!-- AC:END -->
