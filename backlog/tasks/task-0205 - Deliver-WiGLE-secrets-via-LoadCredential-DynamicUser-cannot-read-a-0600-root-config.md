---
id: TASK-0205
title: >-
  Deliver WiGLE secrets via LoadCredential= (DynamicUser cannot read a 0600 root
  config)
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-07-02 04:53'
updated_date: '2026-07-03 09:09'
labels:
  - nix
  - security
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
module.nix sets DynamicUser=true (transient UID unknowable at file-creation time) and passes credentialsFile as a plain --config path. load_config_file errors propagate out of main via ?. So the natural /etc/whereami/secrets.toml 0600 root:root is unreadable by the dynamic UID and the daemon exits non-zero at startup; combined with the missing Restart it stays dead, or with Restart it crash-loops. The only working configs are a world-readable secret (leak) or none. Ref nix/module.nix:22, exec-start.nix:4-5, whereamid/src/main.rs:64, config.rs:192-205.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 a 0600 root:root secrets file is readable by the daemon without widening its mode (LoadCredential + read from CREDENTIALS_DIRECTORY)
- [x] #2 no plaintext secret needs world-readable permissions for the service to work
- [x] #3 startup with a valid secrets file logs WiGLE as configured, not the not-configured warning
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Batched with 0204-0208; see 0204 for shared VM strategy.

changes:
- module.nix serviceConfig: when cfg.credentialsFile != null, LoadCredential = [ "config:${cfg.credentialsFile}" ] (systemd manager reads the 0600 root:root source, provides a 0400 copy to the DynamicUser via $CREDENTIALS_DIRECTORY tmpfs).
- exec-start.nix: new param credentialsFromSystemd ? false. system module passes true -> emits --config ${CREDENTIALS_DIRECTORY}/config (DOCUMENTED ExecStart mechanism, verified in systemd 258 man systemd.exec: use ${CREDENTIALS_DIRECTORY}/mycred). hm keeps the plain --config <path> (user manager reads the user 0600 file; no widening).
- main.rs: add affirmative info!("WiGLE credentials configured - remote lookups enabled") on the is_configured() branch (symmetry with the existing not-configured warn). Satisfies AC#3 literal "logs WiGLE as configured" + observability. Rust change -> full gate (e2e/clippy/fmt).
- No load_config_file change (takes any path).

VM (machine node): environment.etc."whereami/secrets.toml" mode=0600 user/group root, DUMMY wigle creds (AID-dummy...); services.whereami.credentialsFile=that path. Assert: journal has NO "config file not found", HAS "WiGLE credentials configured", NOT "WiGLE credentials not configured"; source file still stat 0600 root:root (unwidened). Bite: broken cred plumbing -> "config file not found" appears -> assert fails. NOTE: environment.etc routes text through the world-readable nix store; only OK because creds are DUMMY (comment it).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carry from task-0209: nix/vm-test.nix now exists (pkgs.testers.runNixOSTest, 2 nodes: machine + nonloopback). Extend THAT test here rather than writing a new one. Gotchas learned:
- Node names become Python variables in the test script; never name a node after a Python keyword (`nonlocal` broke the mypy type-check AND would break at runtime).
- Type=simple socket-activated services report `active` at exec, before the daemon finishes DB migrations / reaches its listener. Gate journal assertions on the daemon's own `control channel: unix socket ready` line (wait_until_succeeds), not just wait_for_unit.
- A failed unit hangs wait_for_unit; poll `systemctl is-failed --quiet` instead (see the nonloopback node).

Implemented (commit effa4a8, fix(nix)).
- exec-start.nix: credentialsFromSystemd param. system -> --config ${CREDENTIALS_DIRECTORY}/config (systemd substitutes it in ExecStart per systemd.exec(5) v258; Nix \${ escaping verified to emit the literal specifier). hm keeps the plain path.
- module.nix: LoadCredential=["config:<path>"] when credentialsFile!=null (via lib.optionalAttrs), credentialsFromSystemd=true. Manager(root) reads the 0600 root:root source, exposes 0400 copy under the credentials tmpfs to the DynamicUser; source mode never widened.
- main.rs: added affirmative info!("WiGLE credentials configured - remote lookups enabled") symmetric to the existing not-configured warn (AC#3). Never logs values. This is the ONLY daemon change.

VM (machine node): 0600 root:root dummy secrets via environment.etc (dummy AID-/dummy-key, NEVER real; noted the store-routing caveat in a comment). Assertions PASS: journal HAS "WiGLE credentials configured", NOT "config file not found", NOT "WiGLE credentials not configured"; stat /etc/whereami/secrets.toml == 600 root:root (unwidened).
- Observed log: INFO whereamid: WiGLE credentials configured - remote lookups enabled (from the LoadCredential-delivered file).
- BITE watched: removed credentialsFile from the node -> daemon logged "config file not found at ~/.config/whereami/config.toml" + "WiGLE credentials not configured" -> subtest FAILED ("grep config file not found unexpectedly succeeded"). Reverted.

Gate: nix flake check exit=0 (6 checks); just e2e 312/9 + clippy + fmt clean (one pre-existing timing-flaky shutdown test on the 1st run, green on rerun; unrelated to this change).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Deliver WiGLE secrets to the DynamicUser system service via systemd LoadCredential, so a 0600 root:root secrets file works without widening it.

Changes:
- module.nix: LoadCredential=["config:<path>"] when credentialsFile!=null; the manager (root) reads the source and exposes a 0400 copy under the credentials tmpfs to the random DynamicUser UID.
- exec-start.nix: credentialsFromSystemd param -> system passes --config ${CREDENTIALS_DIRECTORY}/config (systemd substitutes it in ExecStart; documented in systemd.exec(5)); hm keeps the plain path (the user manager reads its own 0600 file).
- main.rs: affirmative info!("WiGLE credentials configured") counterpart to the existing not-configured warning (AC#3 observability); never logs values.

User impact: the natural /etc/whereami/secrets.toml 0600 root:root now works; no world-readable secret (leak) or empty-config requirement.

Tests: nix flake check green; VM subtest (0600 root:root dummy secrets via LoadCredential) asserts the journal shows "WiGLE credentials configured", NOT "config file not found"/"not configured", and the source stays 0600 root:root. Bite: dropping credentialsFile -> "config file not found" reappears, subtest fails. just e2e 312/9 + clippy + fmt clean.
<!-- SECTION:FINAL_SUMMARY:END -->
