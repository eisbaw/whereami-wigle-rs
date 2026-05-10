---
id: TASK-0079
title: Move CLI to clap or commit to argv-scraping consistently
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-10 11:02'
updated_date: '2026-05-10 16:39'
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
- [x] #1 Either: CLI uses clap and gets typed --help / --version / per-command help, OR: argv-scraping is documented and consistent (no surprise hidden flags)
- [x] #2 All CLI flags (including --scan-time=no, --json, --from, --to) appear in 'whereami help' output
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add clap as a dep in whereami-client/Cargo.toml (workspace already has it).
2. Replace argv-scraping in whereami-client/src/main.rs with clap derive: a Cli struct with global --json flag, and a Commands enum with one variant per subcommand (locate/scan/stats/debug/history/version) plus subcommand-specific flags.
3. History subcommand: positional [range] + --from --to.
4. Scan subcommand: --scan-time=no -> --no-scan-time on the scan subcommand (clap-friendly form). Keep alias if possible OR document the change loudly.
5. Preserve aliases (locate->l, scan->s, stats->st, debug->d, history->h, version->v).
6. Verify whereami help, whereami scan --help, whereami history --help all expose every flag.
7. Manually exercise: whereami help, whereami history --help, whereami locate (without daemon -> err exit 1).
8. cargo build/test/clippy/fmt; commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Deferred during Phase 6: CLI uses argv-scraping; full clap migration would touch every subcommand and reroute help-text generation. The hidden flags --json and --scan-time=no are the only ergonomic issues; the 'whereami help' text now lists --from/--to for history but still omits the global flags. Re-open when adding the next subcommand makes the argv branches painful.

Migrated whereami CLI from argv-scraping to clap derive.

Cli struct: --json (global), --version/-V (global, manual hook to query daemon).
Commands enum: Locate (alias l), Scan (s), Stats (st), Debug (d), History (h), Version (v).

Legacy invocations preserved:
- whereami (no args) -> locate
- whereami --json scan -> scan with json
- whereami scan --scan-time=no -> modeled as --scan-time <yes|no> ValueEnum
- All single-letter aliases

Exit-code drift fixed: clap defaults to exit 2 on parse errors. Wrapped Cli::try_parse to map parse errors to exit 1 (matches the prior CLI). Help/version still exit 0.

Verified manually via running ./target/debug/whereami help, history --help, scan --help — all flags now appear in help output.

10 new clap-derive smoke tests pin the CLI surface. cargo build/test (157 tests pass)/clippy/fmt all green.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Migrated whereami CLI from raw argv-scraping to clap derive.

Why: --json and --scan-time=no were hidden flags (parsed regardless
of position, undocumented in `whereami help`); --from/--to in the
history subcommand had a hand-rolled while loop. Help text drifted
from code. clap fixes all three with no runtime cost.

What changed:
- whereami-client now depends on clap (already in the workspace lockfile).
- src/main.rs: Cli struct + Commands enum replace the previous match-on-argv ladder.
- Global --json works in any position (clap global flag).
- --version/-V kept as a manual hook so it queries the daemon banner
  (clap auto-version would only print the cargo package version).
- Legacy `--scan-time=no` preserved by modeling it as a value-taking
  flag on the scan subcommand (`--scan-time <yes|no>`, default yes).
- All single-letter subcommand aliases preserved.
- Exit code: clap defaults to 2 on parse errors; we map parse errors
  to 1 to match the prior CLI. Help/version still exit 0.
- 10 unit tests pin the clap surface (default subcommand, --json before
  subcommand, legacy --scan-time=no, --from/--to mutual-exclusivity,
  alias resolution, --version flag, unknown subcommand).

Gotcha: I considered renaming --scan-time=no to a clap-native
--no-scan-time flag, but kept the legacy spelling for byte-compat with
user shell aliases. The trade-off is one slightly weird flag shape
in --help; the alternative was a hidden preprocess of argv before
clap, which is exactly the smell this task asked us to remove.
<!-- SECTION:FINAL_SUMMARY:END -->
