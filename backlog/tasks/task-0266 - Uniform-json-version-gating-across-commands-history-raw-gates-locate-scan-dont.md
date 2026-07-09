---
id: TASK-0266
title: >-
  Uniform --json version-gating across commands (history --raw gates,
  locate/scan don't)
status: To Do
assignee: []
created_date: '2026-07-09 00:34'
labels:
  - refactor
  - wire-format
  - cli
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0133 review LOW-2 (pre-existing, not introduced by 0133). The --json byte-passthrough paths are asymmetric on protocol-version handling: history_fixes_raw runs validate_history_fixes_response -> decode_versioned -> check_version, so a v2 error envelope on 'history --raw --json' synthesizes a client error, whereas 'locate --json'/'scan --json' (which do not validate) pass a v>1 daemon line through byte-identically. Decide the intended contract for --json under an unsupported daemon protocol version and make all commands consistent: either version-gate all --json paths (reject v>SUPPORTED) or none (pure byte passthrough). Currently only the raw-history path gates.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Decision recorded: --json either version-gates uniformly (all commands reject v>SUPPORTED) OR passes through uniformly (no command gates)
- [ ] #2 All --json command paths follow the chosen contract; a test pins the behavior for at least one gating and one non-gating command today (locate/scan vs history --raw)
<!-- AC:END -->
