# ADR-050: NixOS-native with systemd hardening

**Status**: Accepted
**Date**: 2026-05-03

## Context

Target deployment is NixOS. The daemon needs Wi-Fi scan capabilities but should not run as root.

## Decision

Provide: `shell.nix` for development, `nix/package.nix` for building, `nix/module.nix` for deployment as a systemd service with `AmbientCapabilities = CAP_NET_ADMIN`.

## Rationale

- NixOS modules are the idiomatic way to deploy services. Declarative, reproducible, rollback-able.
- `AmbientCapabilities = CAP_NET_ADMIN` grants scan privileges without root. Combined with `DynamicUser = true`, the daemon runs as an ephemeral unprivileged user.
- Hardening (ProtectSystem=strict, PrivateTmp, NoNewPrivileges) limits blast radius if the daemon is compromised.
- `makeWrapper` ensures `iw` and `nmcli` are on PATH regardless of system configuration.

## Consequences

- Non-NixOS users need manual `setcap cap_net_admin+ep` on the binary or run via nmcli (which needs no capabilities).
- StateDirectory puts SQLite in `/var/lib/whereami/` — survives service restarts and upgrades.
- Module options mirror CLI args, so NixOS config and CLI stay in sync.
