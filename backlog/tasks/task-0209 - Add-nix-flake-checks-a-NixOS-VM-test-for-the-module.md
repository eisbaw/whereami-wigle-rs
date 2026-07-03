---
id: TASK-0209
title: Add nix flake checks + a NixOS VM test for the module
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-07-02 04:53'
updated_date: '2026-07-03 07:13'
labels:
  - nix
  - testing
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
flake.nix has no checks output and package.nix has no passthru.tests, so nix flake check only evaluates - runs no tests, no clippy, never boots the module. There is no nixosTest covering the unit. Nothing automated would catch the world-readable DB, the DynamicUser/secrets crash, or the negative-int option crash - all boot-time behaviors a VM test surfaces immediately. Ref flake.nix:18-78, nix/package.nix:57-61.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 nix flake check fails on a clippy error or failing unit test
- [x] #2 a NixOS VM test boots nixosModules.default, enables the service, and asserts it reaches active and answers on 127.0.0.1:4747
- [x] #3 the VM test asserts DB/state-dir permissions are not world-readable
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Root-cause the GIT_REV pure-eval trap (blocks nix flake check from even evaluating packages.default): builtins.fetchGit { url = ./..; } fails in pure eval because the flake source is copied to a non-git store path. Fix by threading the flake self rev (self.shortRev or self.dirtyShortRev or "unknown") from flake.nix into package.nix, removing fetchGit. Preserves the real embedded rev (not a delete); distinct from task-0229 (build.rs rerun-if-changed staleness).
2. Restructure nix/package.nix to expose { package, commonArgs, cargoArtifacts, craneLib, src } so checks reuse the SAME cargoArtifacts; keep packages.default contents identical (same commonArgs/preBuild/postInstall/meta).
3. Add checks flake output (both systems where sensible): cargoClippy (-D warnings), cargoTest, cargoFmt, plus the package build. Confirm nix flake check runs them.
4. AC#1 bites-proof: temporarily inject a clippy error, run the clippy check -> observe FAIL; revert; temporarily break a unit test, run the test check -> observe FAIL; revert. Report observed failures.
5. NixOS VM test checks.x86_64-linux.vm via pkgs.testers.runNixOSTest: boot self.nixosModules.default, enable, allowedUsers=[alice], bind=127.0.0.1:4747; users alice + mallory. Assert (a) socket 0660 root:whereami + /run/whereami exists + adopting-fd log + no self-bind log; (b) mallory EACCES; (c) alice {ok:true} version; AC#2 127.0.0.1:4747 answers + loopback TCP warning; (e) second node bind=0.0.0.0:4747 allowNonlocalBind=false -> unit fails, journal names --allow-nonlocal-bind; 0197: /var/lib/whereami (via /var/lib/private) 0700, aps.sqlite 0600, WAL sidecars 0600. Scanner failure must NOT kill daemon (verified in code: run_scan_loop warns+continues).
6. hm item (d): attempt home-manager-in-VM (needs new test-scoped input); if disproportionately hairy, split to a precise follow-up task and reference it (it is a note-spec item, not an AC).
7. Justfile: add nix-check + vm-test recipes (commented, grouped; NOT in e2e).
8. Run Rust gate (nix develop -c just e2e && fmt) once as a regression guard. Commit per logical unit (fix(nix)/test(nix)); tracker changes in a separate final backlog: commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0197 (state dir/DB perms): the VM/flake test MUST assert the resulting mode bits, which task-0197 could only guarantee declaratively (StateDirectoryMode/UMask) + via the daemon backstop, NOT runtime-verify locally. Specifically assert:
- system unit: /var/lib/whereami (following the DynamicUser /var/lib/private symlink) is 0700, and aps.sqlite is 0600 (WAL sidecars -wal/-shm also 0600).
- hm unit: %h/.local/share/whereami is 0700 and aps.sqlite is 0600.
This closes task-0197 AC#1s disclosed nix-path verification gap. Also a good place to assert the real-binary COMPOSITION (main() ensure_private_dir before Database::open), which 0197 verified only at unit level.

forward-carried from task-0196: the NixOS VM test must additionally assert:
(a) /run/whereami/whereamid.sock exists with mode 0660 root:whereami (created by the whereamid.socket unit BEFORE the DynamicUser service starts).
(b) a user NOT in services.whereami.allowedUsers gets connection refused / EACCES connecting to the socket.
(c) a user IN allowedUsers gets a valid version response.
(d) hm variant: $XDG_RUNTIME_DIR/whereamid.sock is mode 0600 and the owner query works.
(e) with cfg.bind set non-loopback and allowNonlocalBind=false the daemon refuses to start (daemon-side validate() rejects it; there is NO nix-side IP parse, so config eval succeeds and the daemon exits nonzero).
GOTCHA to verify: module.nix relies on systemd auto-creating the ListenStream parent dir /run/whereami (mkdir_parents on AF_UNIX bind). Assert /run/whereami exists; if systemd does NOT create it, add a tmpfiles rule / RuntimeDirectory.
Also assert the socket-activated service logs "adopting systemd socket-activated fd 3" and does NOT self-bind.

RESUMED predecessor work; verified + finished + committed.

Predecessor had on disk (uncommitted, no build had completed): flake.nix self-rev threading + checks output; package.nix restructured to return {package, checks} sharing cargoArtifacts; nix/vm-test.nix draft; Justfile nix-check/vm-test recipes. All verified against the diff and largely correct.

What I changed/fixed:
- Added `build = package` to checks (plan item 3, "plus the package build"); `nix flake check` confirms checks.x86_64-linux.build evaluates to the SAME drv as packages.default (kqnjnj10...), so content-equivalent and cached.
- VM test bug #1 (real): node was named `nonlocal`, a Python keyword -> `testScriptWithTypes:62: Invalid syntax` at the type-check stage, and it would break the test script at runtime too (node names become Python vars). Renamed node -> `nonloopback`.
- VM test bug #2 (real, observed it bite): the adopting-fd assertion raced daemon startup. A Type=simple socket-activated service reports `active` at exec, before Database::open+migrations and before setup_unix_listener logs "adopting systemd socket-activated fd". Fix: gate all machine-node journal asserts on the daemon's own `control channel: unix socket ready` line (server.rs) via wait_until_succeeds.

Verification (all foreground, KVM at /dev/kvm):
- `nix flake check -L` GREEN (exit 0): 5 x86_64 checks build/clippy/test/fmt/vm. aarch64 omitted (nix 2.31 checks current-system only unless --all-systems).
- AC#1 bites-proof OBSERVED (not green-only): injected clippy::needless_return -> clippy check failed exit 101 (`-D clippy::needless-return implied by -D warnings`); injected failing unit test -> test check `FAILED. 174 passed; 1 failed` exit 101. Both reverted.
- VM test: all 8 subtests green; test script 13.45s, re-run wall ~27s incl driver rebuild+boot. Assertions: socket 0660 root:whereami + /run/whereami exists; adopting-fd logged + NO self-bind; mallory EACCES; alice ok:true; TCP 127.0.0.1:4747 ok:true + loopback warning; /var/lib/whereami 0700 + aps.sqlite/-wal/-shm 0600; nonloopback unit fails naming --allow-nonlocal-bind.
- Scanner failure in VM (no wlan0) only warns+continues; daemon stayed up and served (confirmed).
- Regression: `nix develop -c just e2e` = 9 binaries / 312 tests + clippy PASS; `just fmt` clean.

Gotchas: dirty-tree embedded rev is `aa014bf-dirty` (self.dirtyShortRev); fallback chain covers clean/dirty/no-git. Pre-existing harmless `warning: unknown flake output homeManagerModules` (nix has no schema for it; unchanged by me).

hm item (d): NOT done here (needs a test-scoped home-manager flake input; disproportionate). Filed task-0234 with precise ACs (self-bind socket 0600, self-bind vs socket-activation, %h DB perms). Forward-carried notes to 0204-0208 (extend this VM test) and 0229 (GIT_REV embedding).

Commits: eee3816 feat(nix) checks infra + self-rev; 77735b4 test(nix) VM test + Justfile recipes.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Add nix flake checks + a NixOS VM test that machine-verifies the module.

Why: `nix flake check` previously only evaluated — it never built the package, ran clippy/tests, or booted the module, so the world-readable-DB / socket-perms / negative-option classes of bug were undetectable in CI.

What changed:
- flake.nix gains a `checks` output: build (= the shipped package), clippy (--deny warnings), test, fmt, and a KVM NixOS VM test (x86_64-linux only). package.nix now returns {package, checks} sharing one crane cargoArtifacts (deps built once).
- Replaced package.nix's `builtins.fetchGit` GIT_REV mechanism (which aborts under nix flake check pure-eval) with self-rev threading from flake.nix (self.shortRev / dirtyShortRev / "unknown"). Package contents are byte-identical; only the rev SOURCE changed.
- nix/vm-test.nix boots nixosModules.default under two nodes and asserts the 0196/0197 guarantees end-to-end: socket 0660 root:whereami under /run/whereami, daemon adopts the socket-activated fd (no self-bind), non-member EACCES / member ok:true, opt-in TCP on 127.0.0.1:4747 + its warning, state dir 0700 with aps.sqlite/-wal/-shm 0600, and a non-loopback bind refused by the daemon's validate().
- Justfile: `nix-check` and `vm-test` recipes, grouped and kept OUT of the fast `e2e` gate.

User impact: `nix flake check` now genuinely gates the security-critical module behaviour on every change.

Tests / evidence: nix flake check green (5 checks). AC#1 proven to bite — an injected clippy lint and an injected failing test each fail their check (exit 101). VM test all-green (8 subtests, ~13s script). Regression: just e2e = 312 tests across 9 binaries + clippy; fmt clean.

Risks / follow-ups: home-manager user-service VM coverage deferred to task-0234 (needs a test-scoped home-manager flake input). Pre-existing harmless "unknown flake output homeManagerModules" warning is unchanged.
<!-- SECTION:FINAL_SUMMARY:END -->
