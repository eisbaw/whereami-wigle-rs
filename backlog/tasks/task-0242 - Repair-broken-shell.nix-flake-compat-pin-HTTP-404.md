---
id: TASK-0242
title: Repair broken shell.nix flake-compat pin (HTTP 404)
status: Done
assignee: []
created_date: '2026-07-05 03:42'
updated_date: '2026-07-05 04:00'
labels:
  - nix
  - tooling
dependencies: []
references:
  - shell.nix
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The repository workflow says to use shell.nix when present, but nix-shell currently exits 1 during evaluation because shell.nix fetches the pinned edolstra/flake-compat archive at commit 35bb57c0c8d8b4f45571e8f20c7571aa6df0a9ae and GitHub now returns HTTP 404. nix-prefetch-url reproduces the 404; the locked native nix develop shell still works. This was discovered while validating TASK-0210 and predates that change. Repair or retire the compatibility wrapper at its root so the documented/required legacy entry point works again.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 nix-shell evaluates shell.nix successfully from a clean checkout and can run a Just recipe inside the project dev environment
- [x] #2 shell.nix resolves to the same default dev shell/tool inputs as flake.nix without a dead or mutable external fetch
- [x] #3 an automated regression check fails if the legacy shell entry point can no longer evaluate
- [x] #4 README/development workflow accurately states the supported nix-shell and nix develop entry points
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Replace the dead flake-compat fetch in shell.nix with direct resolution of the repository git flake and return devShells.${builtins.currentSystem}.default.
2. Add shell.nix evaluation to the existing Justfile e2e gate (inline, not a narrow one-off recipe) and update its doc comment.
3. Reconcile README and PRD development-shell documentation so nix-shell and nix develop are accurately described as the same locked environment.
4. Validate nix-shell can run a Just recipe, Justfile list/summary remain clean, and the full e2e gate passes from the repaired shell.
5. Run mandatory parallel QA/architecture review, fix every blocker, complete the tracker, rerun e2e, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Reproduction on 2026-07-05: nix-shell --run "just fmt" exits 1; nix-instantiate of builtins.fetchTarball exits 1; nix-prefetch-url --unpack for the exact pinned archive reports HTTP 404. Both mandatory TASK-0210 reviewers classified this as a separate prioritized tooling defect.

Replaced the dead flake-compat bootstrap with builtins.getFlake over the Git checkout and selected the flake default devShell directly. The existing e2e recipe now safely instantiates shell.nix before Rust tests/clippy; strict whole-derivation evaluation is intentionally avoided because forcing mkShell cyclic attributes stack-overflows. The repaired shell exposed a deterministic transport-test defect: nix-shell TMPDIR made the old Unix socket path 113 bytes, above Linux sun_path capacity. The test helper now keeps the same tag/PID/full-nanosecond uniqueness in a compact hexadecimal component. Supported boundary is documented: Git checkout plus flakes-enabled Nix; source tarballs/store paths/old flakes-disabled Nix and checkout paths requiring URI escaping are not supported.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Restored the mandated nix-shell workflow without an extra bootstrap fetch or duplicated dev-shell definition. Legacy nix-shell and native nix develop resolve the identical flake-locked derivation. Added a safe shell-instantiation regression to the hygienic existing e2e gate, reconciled README/PRD, and fixed the Unix-socket test path length exposed by the repaired shell. Mandatory QA and architecture gates returned GO. Validation passed legacy and native 417-test E2E suites with strict clippy, formatting, Justfile list/summary, warm-offline equality, and 130/130 stressed transport-test runs with complete cleanup.
<!-- SECTION:FINAL_SUMMARY:END -->
