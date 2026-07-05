---
id: TASK-0200
title: Warn when the WiGLE credentials file is group/world-readable
status: Done
assignee: []
created_date: '2026-07-02 04:52'
updated_date: '2026-07-05 01:47'
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
- [x] #1 starting the daemon with a world/group-readable config file emits a clear warning naming the file and the risk (mode & 0o077 != 0)
- [x] #2 a 0600 config file produces no warning
- [x] #3 docs instruct users to keep the file at 0600
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Open the config once, inspect permissions on the opened descriptor, and warn without mutating the file when group/other bits are set.
2. Add deterministic tests for insecure and owner-only modes, including a real daemon-startup assertion and symlink target semantics.
3. Document mode 0600 in the README and user-facing config option/help text.
4. Run the full Nix end-to-end suite and mandatory parallel QA/architecture gates before commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
2026-07-05 pre-implementation review: QA and architecture independently recommend checking File::metadata on the same descriptor used to read, avoiding an exists/stat/read race and correctly evaluating symlink targets. Missing files retain warn-and-default behavior; metadata inspection failure is advisory and must not expose credential values.

2026-07-05 verification: real daemon startup covers 0644 warning and 0600 silence; focused unit coverage includes 0640, 0604, 0600, 0400, symlink-target metadata, secret non-leakage, and escaped newline/ESC path rendering. Mandatory QA and architecture gates both returned GO after the Nix LoadCredential wording and log-control-character findings were fixed. Final pre-tracker nix develop -c just e2e passed 388 tests and strict all-target Clippy.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented an advisory WiGLE credentials permission check on the same open file descriptor used for parsing, eliminating the prior exists/stat/read race. Group or world permission bits now produce a clear, escaped-path startup warning without logging credentials, changing permissions, or rejecting the file; owner-only 0600/0400 remains quiet and metadata inspection failure warns while loading continues. Added focused tracing and real-daemon regressions, including symlink targets and hostile path characters, and documented owner-only permissions in CLI, README, Nix option text, and ADR-041.
<!-- SECTION:FINAL_SUMMARY:END -->
