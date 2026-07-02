---
id: TASK-0183
title: whereami --json fabricates fields and exits 0 on errors
status: To Do
assignee: []
created_date: '2026-07-02 04:49'
labels:
  - correctness
  - cli
  - protocol
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The CLI --json path re-serializes the parsed client struct (all #[serde(default)], no skip_serializing_if) instead of emitting the daemon's raw response line. On a daemon error {ok:false,error:...} it prints Null-Island lat/lon 0.0/0.0 the daemon never sent, injects error:null on success, drops unknown fields, and exits 0 (the json arm is matched before the !ok exit-1 arm). run_version also prints a non-JSON 'cli: ...' line to stdout under --json when the daemon is down. Docs (protocol.md:204, README:47, main.rs:43) all promise raw passthrough. Ref whereami-client/src/main.rs:124-134, lib.rs:45-78,326.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 whereami --json <cmd> output is byte-identical to the daemon response line for both ok and error responses (use the existing raw_command string)
- [ ] #2 --json exits non-zero when the daemon returns ok:false or is unreachable, matching human mode
- [ ] #3 stdout under --json is always valid JSON, including version with the daemon down
- [ ] #4 a test pins that no field absent on the wire appears in --json output
<!-- AC:END -->
