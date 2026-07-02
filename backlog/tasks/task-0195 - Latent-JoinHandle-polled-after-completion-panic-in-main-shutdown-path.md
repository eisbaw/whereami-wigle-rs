---
id: TASK-0195
title: Latent JoinHandle-polled-after-completion panic in main shutdown path
status: To Do
assignee: []
created_date: '2026-07-02 04:51'
labels:
  - correctness
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The shutdown select has an arm '_ = &mut server_handle => {...}' that consumes the handle output; the drain block then awaits the same handle again (let _ = server_handle.await). tokio panics on a second poll after the output was taken. Today masked by the notify lost-wakeup bug (arm only fires on bind failure at t~0, and the drain times out on scan_handle.await first). Fixing the shutdown race, task-0143, or a mid-life server panic coinciding with parked loops arms it, turning 'server exited' into exit 101. Ref whereamid/src/main.rs:174-199.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 when the server task exits on its own (e.g. bind failure) the daemon shuts down cleanly with a non-panic exit
- [ ] #2 no JoinHandle in main.rs can be polled twice after completion (structurally: Option-wrap or JoinSet)
<!-- AC:END -->
