---
id: TASK-0079
title: Move CLI to clap or commit to argv-scraping consistently
status: To Do
assignee: []
created_date: '2026-05-10 11:02'
updated_date: '2026-05-10 14:15'
labels:
  - cli
  - refactor
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Daemon uses clap. CLI uses raw argv scraping (whereami-client/src/main.rs). The CLI's --json and --scan-time=no are parsed via args.iter().any(|a| ...) regardless of position; --scan-time=no is undocumented (not in 'whereami help'); 'whereami history' has hand-rolled --from/--to parsing. The split causes help-text drift and inconsistent error messages. Pick one and commit. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Either: CLI uses clap and gets typed --help / --version / per-command help, OR: argv-scraping is documented and consistent (no surprise hidden flags)
- [ ] #2 All CLI flags (including --scan-time=no, --json, --from, --to) appear in 'whereami help' output
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Deferred during Phase 6: CLI uses argv-scraping; full clap migration would touch every subcommand and reroute help-text generation. The hidden flags --json and --scan-time=no are the only ergonomic issues; the 'whereami help' text now lists --from/--to for history but still omits the global flags. Re-open when adding the next subcommand makes the argv branches painful.
<!-- SECTION:NOTES:END -->
