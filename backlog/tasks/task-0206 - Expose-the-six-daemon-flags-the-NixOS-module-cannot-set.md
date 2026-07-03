---
id: TASK-0206
title: Expose the six daemon flags the NixOS module cannot set
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-07-02 04:53'
updated_date: '2026-07-03 09:08'
labels:
  - nix
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
options.nix (17 options) + exec-start.nix (13 flags) vs config.rs (21 flags): --address-cache-ttl-days, --http-timeout-secs, --nominatim-timeout-secs, --history-retention-days, --history-segment-distance-m, --history-segment-min-duration-secs have no module option and are never emitted. Most significant: --history-retention-days governs how long location PII is kept, silently pinned to the 30-day compiled default with no module knob. Ref nix/options.nix, exec-start.nix:8-24, whereamid/src/config.rs:11-102.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 every whereamid CLI flag has a corresponding services.whereami.* option emitted by exec-start.nix
- [x] #2 services.whereami.historyRetentionDays measurably changes the daemon's --history-retention-days
- [x] #3 a parity check (test or comment) guards against future flag/option drift
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Batched with 0204-0208; see 0204 for shared VM strategy.

Re-derived parity vs CURRENT config.rs = 23 clap flags. exec-start emits 17 (12 fixed + 5 conditional: socket/bind/allow-nonlocal-bind/config/address-approx). Missing 6: --address-cache-ttl-days --http-timeout-secs --nominatim-timeout-secs --history-retention-days --history-segment-distance-m --history-segment-min-duration-secs.

changes:
- options.nix: add the 6 options (camelCase, descriptions lifted from config.rs doc comments incl 0-disables semantics for addressCacheTtlDays/historyRetentionDays). Bounded types from 0207 (addressCacheTtlDays+historyRetentionDays=unsigned; the 4 timeouts/segment=positive).
- exec-start.nix: emit the 6 (fixed list). After this exec-start emits all 23 (socketFlag=true) -> ZERO missing.
- AC#3 parity GUARD (nix/flag-parity.nix flake check): diff whereamid --help long flags (option-header lines only, drop clap --help/--version) vs the --flag tokens exec-start emits for a FULLY-POPULATED dummy cfg (socketFlag=true, credentialsFromSystemd=false). Empty diff = parity; any asymmetry fails the check. Bites BOTH directions (help-only flag = new clap flag unwired; emitted-only = removed clap flag). Wired in flake.nix checks per-system.

VM AC#2: machine node historyRetentionDays=7 -> systemctl cat whereamid.service | grep -- "--history-retention-days 7". Bite for the guard: temporarily drop one flag from exec-start.nix -> check fails (help has it, emitted does not); revert.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from task-0196: options.nix gained socketPath, bind (now nullOr, default null), allowNonlocalBind, and allowedUsers; exec-start.nix now conditionally emits --socket/--bind/--allow-nonlocal-bind via a socketFlag param (fixed list + optionalString appends). Re-derive the missing-flag list against the CURRENT exec-start.nix before implementing — the structure changed and some targeted flags may now be covered.

Forward-carry from task-0209: nix/vm-test.nix now exists (pkgs.testers.runNixOSTest, 2 nodes: machine + nonloopback). Extend THAT test here rather than writing a new one. Gotchas learned:
- Node names become Python variables in the test script; never name a node after a Python keyword (`nonlocal` broke the mypy type-check AND would break at runtime).
- Type=simple socket-activated services report `active` at exec, before the daemon finishes DB migrations / reaches its listener. Gate journal assertions on the daemon's own `control channel: unix socket ready` line (wait_until_succeeds), not just wait_for_unit.
- A failed unit hangs wait_for_unit; poll `systemctl is-failed --quiet` instead (see the nonloopback node).

Implemented (commit b6aa6f2, feat(nix)).
Re-derived parity vs CURRENT config.rs = 23 clap flags (0196 added socket/bind/allow-nonlocal-bind). exec-start emitted 17; added the 6 missing: --address-cache-ttl-days --http-timeout-secs --nominatim-timeout-secs --history-retention-days --history-segment-distance-m --history-segment-min-duration-secs. exec-start now emits ALL 23 -> ZERO missing.

Parity GUARD = nix/flag-parity.nix (flake check, x86_64 only, execs the binary): diffs `whereamid --help` long flags (option-header lines; drop --help/--version) vs the --flags exec-start emits for a fully-populated cfg (socketFlag=true). Empty diff required; bites BOTH directions.
- PASS observed: "flag parity OK: 23 flags match between --help and exec-start.nix".
- BITE observed: dropped --http-timeout-secs from exec-start -> check FAILED "FLAG PARITY DRIFT ... ---http-timeout-secs" (the new-flag-without-option direction). Reverted.

AC#2 VM: machine node historyRetentionDays=7 -> subtest "systemctl cat whereamid.service | grep -F -- '--history-retention-days 7'" PASS.
Gate: nix flake check GREEN (VM 9 subtests + flag-parity + clippy/test/fmt). Rust untouched (e2e/fmt unchanged from 0207 baseline: 312 tests/9 bins).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Exposed the six whereamid flags the module could not set and added a real guard against future flag/option drift.

Changes:
- options.nix + exec-start.nix: add addressCacheTtlDays, httpTimeoutSecs, nominatimTimeoutSecs, historyRetentionDays, historySegmentDistanceM, historySegmentMinDurationSecs (descriptions lifted from config.rs; bounded types per task-0207). exec-start now emits every one of config.rs 23 clap flags — ZERO missing.
- nix/flag-parity.nix (new flake check): diffs the long flags whereamid --help accepts against the --flags exec-start.nix emits for a fully-populated cfg. Empty diff required; fails in BOTH directions (a new clap flag with no module option, or an emitted flag the binary rejects). A hardcoded list could not catch the former; a --help diff does.

User impact: historyRetentionDays (location-PII retention) and the HTTP/segment tunables are now configurable instead of frozen at compiled defaults.

Tests: nix flake check green; flag-parity reports "23 flags match"; VM subtest asserts systemctl cat carries --history-retention-days 7. Guard bite: dropping --http-timeout-secs from exec-start -> "FLAG PARITY DRIFT" naming the flag.
<!-- SECTION:FINAL_SUMMARY:END -->
