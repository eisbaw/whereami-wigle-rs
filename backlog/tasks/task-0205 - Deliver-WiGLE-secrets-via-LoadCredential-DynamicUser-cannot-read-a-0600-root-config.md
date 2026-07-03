---
id: TASK-0205
title: >-
  Deliver WiGLE secrets via LoadCredential= (DynamicUser cannot read a 0600 root
  config)
status: To Do
assignee: []
created_date: '2026-07-02 04:53'
updated_date: '2026-07-03 07:12'
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
- [ ] #1 a 0600 root:root secrets file is readable by the daemon without widening its mode (LoadCredential + read from CREDENTIALS_DIRECTORY)
- [ ] #2 no plaintext secret needs world-readable permissions for the service to work
- [ ] #3 startup with a valid secrets file logs WiGLE as configured, not the not-configured warning
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carry from task-0209: nix/vm-test.nix now exists (pkgs.testers.runNixOSTest, 2 nodes: machine + nonloopback). Extend THAT test here rather than writing a new one. Gotchas learned:
- Node names become Python variables in the test script; never name a node after a Python keyword (`nonlocal` broke the mypy type-check AND would break at runtime).
- Type=simple socket-activated services report `active` at exec, before the daemon finishes DB migrations / reaches its listener. Gate journal assertions on the daemon's own `control channel: unix socket ready` line (wait_until_succeeds), not just wait_for_unit.
- A failed unit hangs wait_for_unit; poll `systemctl is-failed --quiet` instead (see the nonloopback node).
<!-- SECTION:NOTES:END -->
