---
id: TASK-0183
title: whereami --json fabricates fields and exits 0 on errors
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-07-02 04:49'
updated_date: '2026-07-02 17:47'
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
- [x] #1 whereami --json <cmd> output is byte-identical to the daemon response line for both ok and error responses (use the existing raw_command string)
- [x] #2 --json exits non-zero when the daemon returns ok:false or is unreachable, matching human mode
- [x] #3 stdout under --json is always valid JSON, including version with the daemon down
- [x] #4 a test pins that no field absent on the wire appears in --json output
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. lib.rs: eliminate request() duplication - route every typed method through a new parse_line() + per-command X_raw() that reuses the existing request structs and raw_command(). Single source for request bytes and transport.
2. main.rs: add pure json_output(Result<String>)->(String,i32) + synth_error() (serde_json-escaped {ok:false,error}) + emit_json()->! . Under --json each run_X returns emit_json(client.X_raw()); byte-identical passthrough, ok:false/unreachable/non-JSON -> exit 1, stdout always valid JSON.
3. Drop the json/Serialize branch from dispatch(); run_version json path uses emit_json (no cli: banner to stdout).
4. Tests: pin byte-identical passthrough + no absent field injected (ok:true and ok:false fixtures) + transport/non-JSON synth. Run gate, commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented raw passthrough.
- lib.rs: deleted request(); added parse_response_line() + per-command *_raw() reusing the existing request structs and raw_command(). Single source of truth for request bytes + transport between typed and --json paths.
- main.rs: pure json_output(Result<String>)->(line,i32) parses ONLY `ok` for the exit code and passes the daemon line through byte-identically; synth_error() builds a serde-escaped {ok:false,error} for daemon-down / non-JSON. emit_json() prints+exits.
- Removed Serialize import and the json/re-serialize branch from dispatch(); run_version no longer prints the cli: banner to stdout under --json.
Gotcha: `return emit_json(...)` tripped clippy::diverging_sub_expression + rustc unreachable_expression (emit_json is -> !). Fixed by calling emit_json() in statement position (no return) inside `if json {}`; the following dispatch() stays reachable on the json==false path.
Live verify (daemon up): --json locate byte-identical to raw /dev/tcp line (exit 0); --json history from>to passes ok:false through (exit 1); --json version valid JSON (exit 0); human locate unchanged.
Limitation: daemon-DOWN path (AC#3) could not be live-tested without stopping the owner's running daemon; it is the same emit_json/synth path covered by json_output_transport_error_is_synthesized_valid_json.

Review gate (qa-test-runner + mped-architect, both GO): --json version and error passthrough verified byte-identical live against daemon 533c2c8; exit codes 0/1 confirmed; gate 235 tests green, clippy/fmt clean. Boundaries on AC#3 disclosed for the record: (a) CLI-side usage errors print to stderr with empty stdout (deliberate, TASK-0228); (b) daemon accepts-then-closes mid-write dies 141 via SIGPIPE instead of synthesized JSON (documented at the SIG_DFL site, task-0199 comment); (c) a malformed-but-ok:true line passes through with exit 0 by design (--json is a dumb relay, no schema validation). Post-review hardening in fc54f0d: if/else at all emit_json sites, transport-error sanitization, render_locate byte-pin.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Fixed `whereami --json` to emit the daemon's response line verbatim instead of re-serializing the parsed typed struct.

Why: every response field is #[serde(default)] with no skip_serializing_if, so the old re-serialize path fabricated Null-Island lat/lon 0.0 on errors, injected error:null on success, dropped unknown forward-compat fields, and (because the json arm matched before the ok:false arm) exited 0 on daemon errors. run_version also leaked a non-JSON "cli:" banner to stdout under --json.

Changes:
- lib.rs: replaced request() with parse_response_line() + per-command *_raw() methods reusing the existing request structs and raw_command(); typed and --json paths now share one source of truth for request bytes and transport.
- main.rs: added pure json_output()/synth_error()/emit_json(); --json prints the daemon line byte-identically, derives the exit code from `ok` only, and synthesizes a serde-escaped {"ok":false,"error":...} on transport error or a non-JSON line so stdout is always valid JSON. ok:false and unreachable exit 1.

User impact: `whereami --json <cmd> | jq` now sees exactly what the daemon sent; error responses exit nonzero; stdout is always valid JSON.

Tests: 7 unit tests (byte-identical ok:true/ok:false passthrough, unknown-field survival, nonzero exit on error, valid-JSON synthesis for transport + non-JSON). Gate: whereami-client 17 (bin) + 7 (lib) tests pass, clippy -D warnings clean, fmt clean. Verified live against the daemon.

Follow-up: protocol.md should document the synthesized transport-error object and byte-identical passthrough (TASK-0213).
<!-- SECTION:FINAL_SUMMARY:END -->
