---
id: TASK-0228
title: 'CLI-side usage errors under --json go to stderr, not stdout JSON'
status: To Do
assignee: []
created_date: '2026-07-02 17:29'
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
- [ ] #1 Documented or implemented: under --json, CLI usage errors (clap parse errors and history from/to pairing) behave consistently and the chosen rule is written down
- [ ] #2 If synthesizing JSON, the message is serde-escaped via synth_error and exit stays nonzero; if keeping stderr, the rule is noted in --help or protocol.md
<!-- AC:END -->
