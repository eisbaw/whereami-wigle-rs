---
id: TASK-0196
title: >-
  Restrict the daemon control channel: any local user can read location history
  and abuse WiGLE creds
status: To Do
assignee: []
created_date: '2026-07-02 04:52'
labels:
  - security
  - privacy
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
run_server binds TCP and dispatches every accepted line with zero peer identification (no SO_PEERCRED/auth). Any local process, any UID, can nc 127.0.0.1:4747 and get history (full stay-point history), locate (live position), scan (surrounding BSSIDs/SSIDs), stats, and resolve - which makes the daemon issue WiGLE calls under the owner's credentials (burn quota + proxy arbitrary lookups). ADR-001 only argued no-TLS-because-localhost; the multi-user threat is unexamined. Aggravator: --bind is a raw String with no loopback guard, so --bind 0.0.0.0:4747 silently exposes all of this to the LAN. Ref whereamid/src/server.rs:312-395, config.rs:12-14, docs/adr/001-tcp-json-lines.md.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 a process under a different UID than the daemon cannot obtain locate/history/scan/stats/resolve results (Unix socket + SO_PEERCRED, or a token from the 0600 config)
- [ ] #2 the owner's own client still works with no manual credential step on a default install
- [ ] #3 binding to a non-loopback address requires an explicit opt-in and emits a warning naming the exposure
- [ ] #4 protocol.md documents the access-control model and the multi-user threat
<!-- AC:END -->
