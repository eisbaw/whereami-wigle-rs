---
id: TASK-0133
title: 'Client-side typed Result<LocateOk, DaemonError> instead of union response'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:21'
updated_date: '2026-07-09 00:35'
labels:
  - refactor
  - wire-format
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/whereami-client/lib.rs:46-78. Client LocateResponse defines 'error: Option<String>' alongside the typed body. Daemon's LocateResponse has no 'error' field. Daemon serializes either LocateResponse{ok:true,...} OR ErrorResponse{ok:false,v,error}; client union-decodes via #[serde(default)]. Fragile coincidence: any future field collision (e.g. daemon adds 'error: ...' to LocateResponse for partial failure) silently mis-parses.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Client-side: enum DaemonResult<T> { Ok(T), Err { v, error } } with #[serde(untagged)] OR explicit dispatch on 'ok' bool field
- [x] #2 Each public method (locate, scan, stats, debug, history, version, resolve) returns Result<TypedResp, DaemonError> where DaemonError carries the error string
- [x] #3 The 'error' field is removed from each typed Response struct
- [x] #4 Existing CLI behavior unchanged (CLI just unwraps to text in the same way)
- [x] #5 Unit tests for each response shape: success path and error path
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. protocol: remove `error: Option<String>` from the 9 typed response structs (Locate/Resolve/Scan/Debug/Version/History/HistoryFixes/Stats/Wipe); keep ErrorResponse. Update module doc.
2. daemon (server.rs): drop the 10 `error: None` construction lines (compile follow-through).
3. client lib: add pub DaemonError{v,error} (Error+Display=error string). Split parse: check_version (empty+version gate, peek v+ok) -> decode_versioned<T> (no ok interp; used by history-fixes shape validator so ok:false envelopes still pass through to --json) and parse_response_line<T> (version gate, then ok==false -> decode ErrorResponse -> Err(DaemonError); else decode T). Public typed methods use parse_response_line. Remove now-dead DaemonResponse trait+error() (field gone); keep ProtocolResponse impls.
4. client main: dispatch downcasts Err to DaemonError (print verbatim string, no prefix) else "error: {e}"; rewrite run_daemon_version error arm identically; drop error:None from bin test fixtures; swap DaemonResponse import for DaemonError.
5. wire.rs: drop error:None from constructors; rework error_envelope test to assert ErrorResponse decodes envelope; add client-dispatch DaemonError tests.
6. client lib tests: per-response success+error(DaemonError) unit tests; confirm --json error envelope passthrough byte-identical.
7. Gate: nix develop -c just e2e + cargo fmt --check. Commit code only.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented (commit a6cbf78, code-only; not pushed).

**DaemonError design chosen:** anyhow-wrapped typed error (NOT a ClientError enum). Methods keep returning anyhow::Result<T>; the ok:false path returns Err(DaemonError{v,error}.into()). Rationale: lowest blast-radius — transport/protocol errors keep full anyhow fidelity, no caller signatures change, and DaemonError.Display == the verbatim daemon string so CLI {:#}/context rendering is unchanged. Callers recover it via downcast_ref::<DaemonError>().

**Dispatch (AC#1):** explicit ok-bool dispatch, not #[serde(untagged)] (untagged would reintroduce the coincidence-matching this task removes). parse_response_line: check_version (empty+version gate, peek v+ok via ResponsePrefix) -> if ok==Some(false) decode ErrorResponse -> Err(DaemonError); else decode T. Version gate runs BEFORE ok-dispatch so an unsupported-v error envelope is still rejected as unsupported (tested).

**Split for --json (invariant #1):** decode_versioned (no ok interpretation) is used by validate_history_fixes_response so an ok:false envelope still decodes (ok=false) and passes through the raw --json path byte-for-byte. --json never touches parse_response_line; it uses *_raw()->raw_command directly. Confirmed by test raw_fix_json_api_preserves_error_envelopes_without_success_fields and json_output_* (byte-identical passthrough).

**AC#3 safety (grep evidence):** `grep -rn "error: Some" whereamid/src/` => 0 hits. All 10 response construction sites in server.rs set error: None (LocateResponse x2, Resolve/Scan/Debug/Version/History/HistoryFixes/Wipe/Stats). Removing the always-None skip-when-none field keeps success bytes byte-identical. Verified by wire.rs shape/reserialize tests still passing.

**Removed dead code:** DaemonResponse trait (is_ok/error) — with error field gone and ok:false now an Err, both accessors were dead. ProtocolResponse impls kept via impl_protocol_response! macro.

**Gate:** cargo test 804 passed / 0 failed / 0 ignored (client lib 35, client bin 40, protocol_version 2, sigpipe 1, version_flags 2, wire 34, whereamid lib 209, whereamid main 465, proptests 12, shutdown 4). clippy --all-targets -D warnings clean. cargo fmt --check clean.

**Honest limit / gotcha (not byte-identical in one unreachable case):** old dispatch used resp.error().unwrap_or("unknown error"). New path has no "unknown error" fallback — a malformed {"ok":false,"v":1} with no error key now fails ErrorResponse decode and prints a transport-style "error: parsing daemon error response..." instead of "unknown error". Unreachable via the real daemon (error_json always sets error) and strictly more fail-fast; "byte-identical CLI" holds for all well-formed daemon responses.

**Reviews:** qa-test-runner (804/0) and mped-architect both clean, no blocking issues. Architect confirmed dispatch sound against daemon (only ok:false path is error_json->ErrorResponse; return_last_fix_or_error returns ok:true or ErrorResponse, never an ok:false typed struct). Non-blocking note: version-gate-before-ok-dispatch means a v1 client rejects even a v2 daemon's error-envelope message as unsupported — deliberate conservative choice, pre-existing property, tested; not filing a task.

## Review gate (orchestrator, independent) — GO/GO
- QA (re-run): 804/804 tests, TWO consecutive runs byte-identical (zero flake); clippy 0 under -D warnings; fmt clean. New daemon_error_dispatch_tests + reworked wire.rs error-envelope test + --json passthrough tests all present/green.
- Architecture (read-only): all 5 invariants CONFIRMED with evidence — (1) success wire byte-identical (grep error:Some = 0 hits; removed field was skip-when-none; wire_shape byte-pins hold); (2) --json error-envelope passthrough byte-identical (routes emit_json(*_raw), never parse_response_line; history_fixes_raw switched to non-dispatching decode_versioned — the one real risk, avoided); (3) CLI text+exit code identical via downcast (verbatim daemon string vs error: prefix); (4) fail-fast improved; (5) version-gate-before-dispatch pre-existing, not a regression.
- Both disclosed edges ACCEPTABLE: malformed {ok:false} with no error key is UNREACHABLE (error_json always sets error; ErrorResponse.error is non-Option String) and now fail-fast; version-gate ordering pre-existing.
- LOW-2 (pre-existing --json version-gate asymmetry: history --raw gates, locate/scan pass through) filed as follow-up TASK-0266.
- LOW-1 (vestigial "error" in two wire_shape test names) left as-is: reviewer confirmed not a defect — the !json.contains("error") assertions still validly guard against re-introducing a serialized error key.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Made daemon application errors a first-class typed error on the client, replacing the fragile in-band error union.

Before: every daemon line decoded straight into a typed response struct that also carried error: Option<String> (serde default), so an error envelope {ok:false,v,error} decoded into e.g. LocateResponse with defaulted fields + error populated — a coincidence that would silently mis-parse if the daemon ever added an error field to a success response.

What changed:
- whereami-protocol: removed the error field from all 9 typed response structs (ErrorResponse unchanged). Byte-safe: the daemon set error:None at every success site (grep error:Some = 0) and the field was skip-when-none, so success-response bytes are byte-identical.
- whereami-client: parse_response_line now peeks the ok bool (ResponsePrefix) after the version gate; ok:false decodes ErrorResponse and returns Err(DaemonError{v,error}.into()) — an anyhow-wrapped typed error (chosen over a ClientError enum for lowest blast radius: transport/protocol errors keep full anyhow fidelity, no caller signatures change). DaemonError is a real std::error::Error whose Display is the verbatim daemon string. Explicit ok-dispatch, NOT serde(untagged) (which would reintroduce the coincidence).
- CLI: dispatch()/run_daemon_version() downcast to DaemonError to preserve the exact text and exit codes.
- --json passthrough UNTOUCHED: *_raw() paths return the raw line verbatim (incl. ok:false envelopes); history_fixes_raw switched to a non-dispatching decode_versioned so its validation no longer diverts error envelopes.

Why: removes a silent-misparse hazard and makes the daemon-error contract explicit.

User impact: none for well-formed responses — CLI text + exit codes byte-identical; --json byte-identical. One unreachable edge (malformed ok:false with no error key) now fails fast instead of printing "unknown error".

Tests: 804 workspace (was 795), two independent runs zero flake; clippy 0; fmt clean. New per-shape success+error dispatch tests, reworked wire.rs error-envelope decode test, --json error-envelope passthrough tests.

Follow-up: TASK-0266 (pre-existing --json version-gate asymmetry across commands).

Commit a6cbf78.
<!-- SECTION:FINAL_SUMMARY:END -->
