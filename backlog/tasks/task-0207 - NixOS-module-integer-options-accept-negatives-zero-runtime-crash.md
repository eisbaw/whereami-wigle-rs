---
id: TASK-0207
title: NixOS module integer options accept negatives/zero -> runtime crash
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
All numeric options in options.nix use lib.types.int, which accepts negatives and zero. The daemon rejects these at runtime in Args::validate (e.g. --pending-max-attempts must be >0, --top-n must be >0), so services.whereami.topN=0 type-checks, builds, and then crash-loops on boot - the worst time to learn. Interacts with the missing Restart to hot-loop. Ref nix/options.nix:44-102, whereamid/src/config.rs:112-162.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 topN=0 or any <=0 count fails nixos-rebuild evaluation, not at runtime (ints.positive/unsigned/between)
- [x] #2 option numeric bounds mirror the daemon's validate() invariants
- [x] #3 valid configurations continue to evaluate unchanged
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Batched with 0204-0208; see 0204 for shared VM strategy.

Mirror Args::validate() EXACTLY (checked each against config.rs types + validate()):
- ints.positive (>=1): scanIntervalFast, scanIntervalSlow, debounceWindow, debounceThreshold, topN, pendingInterval, pendingMaxAttempts, notFoundTtlDays, httpTimeoutSecs, nominatimTimeoutSecs, historySegmentDistanceM, historySegmentMinDurationSecs.
- ints.unsigned (>=0): scanFastDuration (u64, NO validate check -> 0 legit), dailyLimit (u32, NO validate check -> 0 legit), addressCacheTtlDays (validate <0 bail, 0 disables), historyRetentionDays (validate <0 bail, 0 disables).
- Do NOT invent upper bounds (validate has none; clap i32/u32 overflow stays a runtime parse guard -> note residual).

Cross-field: debounceThreshold <= debounceWindow -> shared nix/assertions.nix returning an assertions list, imported into module.nix AND hm-module.nix config (message names both options). DRY single source.

Bites-proof (REQUIRED, 0209-style; eval-only, no VM boot): temporarily set topN=0 in a vm-test node -> nix build .#checks.x86_64-linux.vm fails at eval with a type error naming topN; separately debounceThreshold=11 debounceWindow=10 -> assertion failure naming both. Observe, revert, report exact messages. AC#3: existing valid nodes still evaluate.

LIMIT: hm-module assertion cannot be evaluated here (no home-manager flake input; task-0234). Prove the mechanism via the NixOS module; forward-carry hm verification to 0234.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carry from task-0209: the NixOS VM test (nix/vm-test.nix) is the natural place to prove this. The nonloopback node already demonstrates the pattern — boot the module with a bad option and assert the unit fails (poll `systemctl is-failed`, since a failed unit hangs wait_for_unit). options.nix uses lib.types.int with no lower bound; the daemon's clap value_parser + Args::validate() is the current runtime guard. A VM assertion (boot with e.g. debounceWindow=0 or a negative int, expect the unit to fail naming the offending flag) would lock the guard in. Same runNixOSTest gotchas apply: gate journal reads on the daemon `... unix socket ready` line; do not name a node after a Python keyword.

Implemented (commit 0ae4eee, fix(nix)). options.nix numeric types now mirror Args::validate():
- ints.positive: scanIntervalFast, scanIntervalSlow, debounceWindow, debounceThreshold, topN, pendingInterval, pendingMaxAttempts, notFoundTtlDays.
- ints.unsigned: scanFastDuration + dailyLimit (validate() has NO check on these -> 0 legit; mirrored, did NOT invent stricter bounds), addressCacheTtlDays + historyRetentionDays (added in 0206; 0 disables).
- Cross-field debounceThreshold<=debounceWindow -> shared nix/assertions.nix imported by module.nix AND hm-module.nix (DRY).
- Upper bounds intentionally omitted (validate() has none; clap u32/i32 parse is the residual runtime guard for absurd values).

BITES-PROOF (0209-style, eval-only, no VM boot):
1. topN=0 in machine node -> nix build .#checks...vm FAILED at eval: "A definition for option `nodes.machine.services.whereami.topN' is not of type `positive integer, meaning >0'". Reverted.
2. debounceThreshold=11 (debounceWindow=10) -> FAILED at eval: "Failed assertions: - services.whereami.debounceThreshold (11) must be <= services.whereami.debounceWindow (10): ...". Reverted.
Also option-level: opts.topN.type.check 0=false, .check(-1)=false, .check 10=true; dailyLimit.type.check 0=true, (-1)=false.

Gate: nix flake check GREEN (8 VM subtests, valid configs evaluate); just e2e 312 tests/9 bins + clippy clean; just fmt clean.

LIMIT: hm-module.nix assertion cannot be eval-verified here (no home-manager flake input); mechanism proven via the NixOS module. Forward-carry to 0234.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Bounded the NixOS/home-manager module numeric options so out-of-range values fail evaluation instead of crash-looping the daemon at boot.

Changes:
- options.nix: lib.types.int -> ints.positive / ints.unsigned, mirroring Args::validate() (config.rs) exactly. positive (>=1) for the count/interval options validate() bails on; unsigned (>=0) for scanFastDuration + dailyLimit (validate() has NO check, so 0 is legitimate — the module must not be stricter) and the two *-days that disable at 0.
- nix/assertions.nix (new, shared by both modules): the cross-field debounceThreshold <= debounceWindow invariant as a module assertion (single source of truth).
- No upper bounds (validate() has none; clap u32/i32 parse remains the runtime guard for absurd values).

User impact: services.whereami.topN=0 and similar now fail nixos-rebuild with a clear message naming the option, not a boot-time crash-loop.

Tests: nix flake check green (valid configs evaluate). Eval bites observed: topN=0 -> "not of type positive integer, meaning >0"; debounceThreshold=11 > debounceWindow=10 -> assertion naming both options.

Residual: the hm-module assertion cannot be evaluated in this repo (no home-manager flake input); mechanism proven via the NixOS module, forward-carried to task-0234.
<!-- SECTION:FINAL_SUMMARY:END -->
