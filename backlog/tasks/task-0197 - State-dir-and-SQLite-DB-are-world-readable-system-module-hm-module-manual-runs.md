---
id: TASK-0197
title: >-
  State dir and SQLite DB are world-readable (system module + hm-module + manual
  runs)
status: Done
assignee:
  - '@claude'
created_date: '2026-07-02 04:52'
updated_date: '2026-07-02 23:20'
labels:
  - security
  - privacy
  - nix
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
nix/module.nix sets StateDirectory=whereami with no StateDirectoryMode (systemd default 0755) and no UMask (default 0022 -> aps.sqlite 0644). The daemon never chmods. The system module is incidentally protected only because DynamicUser puts state under /var/lib/private (0700); the hm-module (db under ~/.local/share) and any --db-relocated/manual run leave aps.sqlite - which holds the fixes location-history table, last_fix, and every BSSID/SSID seen - readable by other local users, even when the daemon is stopped. Ref nix/module.nix:22-23, whereamid/src/main.rs:57-61, db.rs:104-105.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 on a fresh start the DB file and its directory are not group/other readable (verified by mode bits) on system, hm, and manual paths
- [x] #2 set StateDirectoryMode=0700 and UMask=0077 in both nix units; daemon creates the state dir 0700 and DB 0600
- [x] #3 existing databases are tightened (or a warning emitted) on startup
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. nix/module.nix serviceConfig: add StateDirectoryMode = "0700" and UMask = "0077".
2. nix/hm-module.nix Service: add UMask = "0077" (no StateDirectory here; daemon creates the dir).
3. Daemon-side (covers manual runs + backstops both nix paths):
   - main.rs: replace bare create_dir_all with ensure_private_dir() using DirBuilder::new().recursive(true).mode(0o700) (mode applies only to components it creates; pre-existing shared parents untouched). After ensuring, warn_if_dir_accessible() warns (no chmod) if the DB parent is group/other-accessible.
   - db.rs Database::open(): call tighten_db_file_permissions(path) right after Connection::open, BEFORE the journal_mode pragma, so -wal/-shm sidecars inherit 0600. set_permissions main file to 0600 + tighten any pre-existing -wal/-shm (ignore NotFound). chmod failure -> warn + continue. open_memory() untouched.
4. Tests:
   - db.rs: fresh open -> file mode 0600; pre-existing loose (0644) DB reopened -> 0600; tighten helper covers main+sidecars+NotFound (unique temp dir, hand-rolled like shutdown.rs).
   - main.rs test module: ensure_private_dir creates nested components 0700.
5. Validate nix: nix-instantiate --parse on both module files; nix develop -c true. Gate: nix develop -c just e2e + fmt.
6. AC#1 honesty: manual path runtime-verified by unit tests; system/hm paths guaranteed by StateDirectoryMode/UMask + daemon backstop but NOT VM-testable until task-0209 -> forward-carry note. Commit fix(daemon): then fix(nix): (or merged).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented across two commits: a965ae9 (daemon-side) + d1b81b2 (nix units).

Verification honesty for AC#1 (names system, hm AND manual paths):
- Manual path: RUNTIME-VERIFIED by unit tests — db.rs open_creates_db_file_0600 (fresh file mode 0600), db.rs open_tightens_preexisting_loose_db (0644->0600 on reopen), db.rs tighten_db_file_permissions_covers_main_and_sidecars, main.rs ensure_private_dir_creates_nested_components_0700 (nested dirs 0700, pre-existing base untouched).
- System + hm paths: the daemon-side backstop (create dir 0700 + chmod DB 0600 on every open) runs regardless of systemd settings, and that daemon behavior IS the code the unit tests exercise — so the SUBSTANCE of AC#1 (DB+dir not group/other readable on all three paths) is genuinely satisfied at runtime by tested code. The nix-declarative layer (StateDirectoryMode/UMask) is defense-in-depth and is NOT independently runtime-verified here: asserting the actual mode bits under real systemd requires a VM/flake check, which is task-0209s scope. Forward-carried to 0209.
- Composition gap (disclosed): main() wiring ensure_private_dir(parent) before Database::open(db) is verified by reading + compile, not by a spawn-the-binary integration test (steer scoped tests to unit level; 0209 VM test will cover the real-binary end-to-end). Each half is unit-tested independently.
- warn_if_dir_accessible is exercised but its warn side-effect is not assertion-tested (project has no tracing-capture harness); logic is a single mode&0o077 check.

AC#2 nuance: hm-module has no StateDirectory, so StateDirectoryMode is N/A there (only UMask=0077 set); the daemon creates that dir 0700 instead. module.nix has both StateDirectoryMode=0700 and UMask=0077.

Nix validation run: nix-instantiate --parse on both module files (PARSE_OK), nix develop -c true (OK), nix flake show (evaluates; nixosModules.default + homeManagerModules present). StateDirectoryMode/UMask are standard systemd serviceConfig freeform string options.

Gate green: 279 tests (7+22+1+85+151+11+2), clippy clean, fmt clean.

Review gate (orchestrator, 2026-07-03): qa-test-runner GO + mped-architect GO. Architect judged the AC#1 disclosed-gap handling honest and sufficient (NOT AC-gaming): manual path mode-bit-verified, system/hm covered by the same tested daemon backstop, nix layer deferred to 0209. In-thread hardening 53aba50 addressed the two review minors: (1) tighten_file_to_0600 now re-stats after successful chmod and warns if the file is still loose (FUSE/network FS can ignore chmod) — fail-closed for a privacy feature; (2) new test real_wal_sidecar_is_born_owner_only pins the SQLite sidecar mode-inheritance claim against a REAL SQLite-created -wal (was previously verified only by reasoning). Post-fix gate: 281 tests green, clippy+fmt clean. Known accepted nits: fresh-creation mode asserts are umask-vacuous under 0077 (the loose-preexisting tests are the real guards); create-then-chmod microsecond window on manual runs under loose umask (rusqlite has no create-mode hook; nix paths have UMask=0077).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed the world-readable state-dir/DB exposure with three layers of defense (task-0197). The SQLite DB holds fixes location-history, last_fix, and every BSSID/SSID seen; on the hm path and any manual/--db-relocated run it was left 0644 in a 0755 dir.

Changes:
1. Daemon backstop (covers manual runs + reinforces both nix paths, runtime-tested):
   - main.rs ensure_private_dir(): DirBuilder::mode(0o700) creates missing dir components owner-only at creation time; pre-existing shared parents left untouched. warn_if_dir_accessible() warns (never chmods) on a group/other-accessible pre-existing dir.
   - db.rs Database::open(): chmod DB file to 0600 on every open BEFORE the journal_mode pragma so WAL -wal/-shm sidecars inherit 0600; also tightens pre-existing sidecars. chmod failure warns and continues (read-only FS still serves reads). open_memory() untouched.
2. nix/module.nix: StateDirectoryMode=0700 + UMask=0077.
3. nix/hm-module.nix: UMask=0077 (no StateDirectory in user units; daemon makes the dir 0700).

User impact: location history is no longer readable by other local users on any path, even when the daemon is stopped.

Tests (green, 279 total): fresh open -> 0600 file; existing loose DB tightened to 0600 on reopen; tighten helper covers main+sidecars+NotFound; ensure_private_dir nests 0700 without touching the pre-existing base.

Verification gap (disclosed, see Implementation Notes): the manual path is mode-bit-verified by unit tests and the daemon backstop guarantees the substance on system+hm paths at runtime; asserting the mode bits under real systemd/home-manager needs a VM check, deferred to task-0209 (forward-carried). AC#2 nuance: hm-module has no StateDirectory, so StateDirectoryMode is N/A there.

Nix validation: nix-instantiate --parse both files (OK), nix develop -c true (OK), nix flake show evaluates.

Commits: a965ae9 (daemon), d1b81b2 (nix).
<!-- SECTION:FINAL_SUMMARY:END -->
