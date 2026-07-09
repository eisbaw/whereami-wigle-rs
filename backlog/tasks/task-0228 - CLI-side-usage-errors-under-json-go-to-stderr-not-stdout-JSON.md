---
id: TASK-0228
title: 'CLI-side usage errors under --json go to stderr, not stdout JSON'
status: Done
assignee:
  - '@myself'
created_date: '2026-07-02 17:29'
updated_date: '2026-07-09 05:48'
labels:
  - cli
  - protocol
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0183 made `whereami --json` always emit valid JSON on stdout for daemon responses and transport errors. But CLI-side argument/usage errors still print to stderr with empty stdout and exit 1 under --json: clap parse errors (unknown flag/subcommand) and the history --from/--to pairing check (run_history: "history: --from requires --to"). This matches clap's behavior and is a defensible reading (usage errors are stderr; only daemon results are stdout-JSON), but a strict reading of "stdout under --json is always valid JSON" would want these synthesized as {"ok":false,"error":...} too. Decide and make it consistent (either document the stderr-for-usage-errors rule, or wrap usage errors into synth_error JSON under --json). Low priority; current behavior is intentional, not a regression.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Documented or implemented: under --json, CLI usage errors (clap parse errors and history from/to pairing) behave consistently and the chosen rule is written down
- [x] #2 If synthesizing JSON, the message is serde-escaped via synth_error and exit stays nonzero; if keeping stderr, the rule is noted in --help or protocol.md
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. DECISION: DOCUMENT the honest split (Framing B, task-lead recommended): under --json, DAEMON results + transport errors are stdout-JSON; ALL CLI usage errors (clap parse errors AND the history from/to pairing check) remain stderr + nonzero exit. Do NOT wrap the pairing check (keeps both usage errors uniform on stderr; more consistent than splitting).
2. Finalize docs/protocol.md (remove the "future decision in TASK-0228" language, state the settled rule).
3. Add a concise note to the --json arg help in whereami-client/src/main.rs.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DECISION: document (do NOT JSON-wrap). Under --json: daemon results + transport errors are stdout-JSON (task-0183); ALL CLI usage errors stay stderr + nonzero exit — clap parse errors (incl. max_fixes range check) exit before dispatch, and the run_history --from/--to pairing check uses fatal()=eprintln!+exit(1) (not JSON-aware). Did NOT JSON-wrap the pairing check (wrapping only that one in-code usage error would make two otherwise-identical usage errors diverge; uniform stderr is the cleaner contract; clap errors can\x27t be intercepted anyway). Documented in protocol.md + the --json flag help + a run_history inline comment. Reviewer CONFIRMED the rule is TRUE (no usage error reaches stdout; precedence: target-resolution transport JSON runs before the pairing check). Dual gate GO/GO.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Documented the --json output-stream contract: daemon responses + transport errors are valid JSON on stdout (task-0183), while CLI USAGE errors (clap arg-parse errors and the history --from/--to pairing check) remain plain-text on stderr with a nonzero exit. Chose to document rather than JSON-wrap — clap parse errors exit before our dispatch and can\x27t be cleanly intercepted, and wrapping only the one in-code usage error would make otherwise-identical usage errors diverge. Documented in protocol.md, the --json help, and an inline comment. Reviewer verified the rule holds (no usage error escapes to stdout). Doc-only; dual gate GO/GO. Commit a279514.
<!-- SECTION:FINAL_SUMMARY:END -->
