---
id: TASK-0104
title: 'parse_iw_output: warn on malformed signal line (parity with nmcli)'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:11'
updated_date: '2026-07-08 18:47'
labels:
  - robustness
dependencies:
  - TASK-0251
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/scanner.rs:252-258. If 'signal: <garbage> dBm' appears mid-block, the line is silently skipped, current_signal stays None, and the BSSID block is dropped at the next BSS boundary. task-0043 fixed this for nmcli; iw parser has the same shape but no warn. Driver corner-cases (e.g. signal printed as 'unknown') would silently drop APs without observability.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 On signal-line parse failure, emit tracing::warn (rate-limited to once per BSS block) with the offending line excerpt
- [x] #2 BSS is still dropped (correct behavior), but operator can see why
- [x] #3 Unit test: parse_iw with one malformed signal line in an otherwise-valid block; assert log captured + BSS dropped
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. iw parser (~898-903): add else branch to signal: arm emitting warn! with offending line excerpt; keep drop behavior (current_signal stays None).
2. nmcli parser (~718-721): change Err(_) arm to warn! before continue (real parity + observability).
3. Add scoped capture subscriber in scanner mod tests (mirror db.rs CapturedEvents), calling install_permissive_global_test_subscriber() first.
4. Test: two-BSS iw text, one malformed signal; assert malformed BSS absent, valid present, WARN captured.
5. Optional nmcli warn test.
6. Gate: cargo test -p whereamid scanner (lib+bin), just e2e, clippy -D warnings, fmt --check. Run new tests twice.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Triaged 2026-07-08 (not implemented — deferred behind task-0251):
1. PREMISE CORRECTION: the task says task-0043 added a malformed-signal warn to the nmcli parser and asks for "parity". But the CURRENT nmcli parser (scanner.rs:~719) silently `continue`s on an unparseable signal with NO warn (comment "Skip rows where signal is missing or unparseable"). So there is no nmcli warn to mirror. The iw-parser observability gap (scanner.rs:~899-903: malformed `signal:` line -> current_signal stays None -> BSS dropped at next boundary, silently) is STILL a real robustness improvement worth doing — just re-frame away from "parity with nmcli" (and consider adding the same warn to the nmcli continue-site for real parity).
2. AC#3 ("assert log captured") is entangled with the tracing-capture flake: parse_iw_output is a pure fn, so verifying its warn! needs a scoped subscriber, and scanner tests run in the whereamid LIB test binary alongside db.rs/trilaterate.rs capture tests -> the same process-global interest-cache clobber task-0248 fixed for the BIN binary and task-0251 tracks for the LIB side. A naive capture test here would be flake-prone. => depends on task-0251 (shared permissive-global test-subscriber reachable from lib-target modules), OR re-scope AC#3 to a deterministic assertion (e.g. assert BSS dropped only, or expose a malformed-line diagnostic).
Recommend: do 0251 first (hoist install_permissive_global_test_subscriber to a lib+bin-reachable test-support module), then this becomes a clean warn+capture task.

Implemented 2026-07-08 (commit d6819dc), left In Progress for the mandatory review gate.

Changes (whereamid/src/scanner.rs):
- iw parser signal: arm (~898): added else branch emitting warn! "iw scan: unparseable signal line {line:?}; AP in this block will be dropped". Drop behavior unchanged (current_signal stays None -> BSS dropped at next boundary). One signal line per BSS = effectively once-per-block, no extra rate-limit machinery.
- nmcli parser (~718): real parity fix per triage — the Err(_) => continue arm now warns "nmcli scan: unparseable signal {:?}; skipping AP row" before continuing. (Title implied nmcli already warned; it did NOT — added it.)
- mod tests: added CapturedEvents scoped subscriber (mirrors db.rs verbatim) + capture_scanner_events() which calls install_permissive_global_test_subscriber() first (task-0251). Two tests: parse_iw_output_warns_and_drops_bss_with_malformed_signal (AC#1/#2/#3) and parse_nmcli_output_warns_and_skips_row_with_malformed_signal (nmcli, nice-to-have).

Gate (nix develop -c, bare cargo not on PATH):
- cargo test -p whereamid scanner: LIB 41 passed / 0 failed; BIN 44 passed / 0 failed. Both new tests pass in BOTH binaries. Ran twice — stable, no flake.
- just e2e: exit 0, all suites pass.
- cargo clippy --all-targets -D warnings: clean.
- cargo fmt --check: clean (needed one fmt pass; the warn! macro line got wrapped).

Gotcha: scanner mod tests use super::* but that did NOT re-export the parent-module private `use std::sync::Arc` — had to add `use std::sync::{Arc, Mutex}` explicitly in the tests block.

AC status (not checked — review gate owns that): AC#1 met (warn with line excerpt, once-per-block), AC#2 met (BSS still dropped, now observable), AC#3 met (unit test asserts capture + drop, deterministic via permissive global).

No follow-ups required; the "parity with nmcli" premise correction from triage is resolved by adding the nmcli warn.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added observability when a scanner parser hits a malformed signal line — previously the whole AP was silently dropped with no log, so an operator could not see why APs vanished (driver corner cases like "signal: unknown dBm").

Changes (commit 16047ce, whereamid/src/scanner.rs):
- parse_iw_output: the `signal:` arm now warns on an unparseable value; drop behavior unchanged (current_signal stays None -> BSS dropped at the next boundary, AC#2). One signal line per block => effectively once-per-block (AC#1). Wording softened per review to not over-assert the drop (a later valid signal line in the same block still keeps the AP).
- parse_nmcli_output: the signal-parse Err arm now warns before `continue` (PREMISE CORRECTION: the task title said "parity with nmcli" implying nmcli already warned — it did NOT, it also silently continued; so genuine parity = warn in BOTH parsers).
- Tests (AC#3): a scanner-local CapturedEvents sink + crate::test_support::install_permissive_global_test_subscriber() (task-0251) for reliable capture in both the lib and bin test binaries; asserts the malformed-signal AP is dropped, the valid one kept, and a WARN naming the offending line is emitted. iw + nmcli tests.

Verification: review qa GO (745 workspace tests; scanner 41 lib / 44 bin, both new tests stable across 6 runs — no tracing-capture flake thanks to task-0251; clippy -D warnings + fmt clean) / mped GO. In-thread review hardening: softened the iw warn wording (M2) so it does not assert a drop it cannot guarantee on multi-signal-line blocks.

Follow-up filed (task-0256): (1) M1 — the WARN re-fires every scan cycle for a chronically-malformed driver (log spam); dedup/demote-to-DEBUG needed. (2) the nmcli `fields.len() < 5 => continue` short-row drop is another silent-vanish site (scanner.rs:702) needing the same observability. Both out of 0104 scope.
<!-- SECTION:FINAL_SUMMARY:END -->
