# ADR-003: Unix domain socket + filesystem-permission access control

**Status**: Accepted
**Date**: 2026-07-03

## Context

The daemon's control channel had no peer authentication (task-0196).
[ADR-001](001-tcp-json-lines.md) chose loopback TCP and argued only that *no TLS
is needed because it is bound to 127.0.0.1* — it considered network exposure but
**not** the multi-user threat. On a shared host, any local user (any UID) could:

- `nc 127.0.0.1:4747` and read `history` (full stay-point history), `locate`
  (live position), `scan` (surrounding BSSIDs/SSIDs), and `stats`;
- issue `resolve`, making the daemon perform WiGLE lookups under the owner's
  credentials — burning the daily quota and proxying arbitrary lookups.

Aggravating factor: `--bind` was a raw string with no loopback guard, so
`--bind 0.0.0.0:4747` silently exposed all of the above to the whole LAN.

We need an access-control mechanism that (a) lets the owner's own client work
with no manual credential step on a default install, and (b) works for the
system deployment, where the daemon runs as a systemd `DynamicUser` (a random
UID with no fixed home) yet the owner's UID must be able to connect.

## Decision

Serve the control channel over a **Unix domain socket**, gated by **filesystem
permissions on the socket** — the Unix-native access-control model. TCP becomes
opt-in.

- **Self-bound (home-manager / manual runs):** the daemon binds
  `$XDG_RUNTIME_DIR/whereamid.sock` (or `/run/whereami/whereamid.sock`), chmods
  it to **0600**, and re-stats to fail closed if the mode did not take. Under
  `$XDG_RUNTIME_DIR` (0700, user-owned) this is single-user by construction.
- **System service (DynamicUser):** systemd **socket activation** creates
  `/run/whereami/whereamid.sock` as `root:whereami` mode **0660** *before* the
  daemon starts and passes it as fd 3. Members of the `whereami` group
  (`services.whereami.allowedUsers`) can connect; the daemon needs no fixed UID.
  When socket-activated the daemon skips unlink/chmod/cleanup (systemd owns the
  file).
- **Stale vs live sockets:** on self-bind, an existing socket is probed by
  connecting. Connect refused ⇒ stale inode from a dead daemon ⇒ unlink and
  bind. Connect succeeds ⇒ a live daemon owns it ⇒ **refuse to start** (never
  steal a live socket). A non-socket file at the path is never clobbered.
- **TCP is opt-in** via `--bind host:port`, off by default. The daemon warns
  loudly when it is enabled and **refuses** a non-loopback bind unless
  `--allow-nonlocal-bind` is also given.
- The daemon logs the connecting peer's `SO_PEERCRED` (uid/gid/pid) at debug
  level for auditability but does **not** enforce same-UID in-process.

## Rationale / alternatives rejected

- **In-process `SO_PEERCRED` same-UID enforcement.** Rejected: it breaks the
  system-service group model, where systemd deliberately hands a group-readable
  socket to a DynamicUser daemon whose UID differs from every caller's. The
  socket's filesystem permissions already decide who may connect, so an
  in-process UID check is both redundant and wrong for the group case.
- **Token in the 0600 config file.** Rejected: under `DynamicUser` there is no
  location both the daemon and the owner can read a shared secret from without a
  group anyway — so we would still need the `whereami` group, at which point the
  group *is* the access control and the token is pure added state (a second
  source of truth to distribute, rotate and leak).
- **Client auto-fallback to `127.0.0.1:4747`.** Rejected: once the daemon no
  longer listens on that port, any local user could bind it and impersonate the
  daemon (an integrity downgrade — the client would trust squatted responses).
  The client therefore never falls back to TCP implicitly; point it at a TCP
  daemon explicitly with `WHEREAMI_ADDR`.

## Consequences

- **Breaking change.** The daemon and client must be upgraded together: the new
  client speaks unix by default and the new daemon no longer listens on TCP
  unless `--bind` is set. An old client can still reach a new TCP daemon via
  `WHEREAMI_ADDR`; a new client can reach an old TCP daemon the same way.
- The daemon is Linux/Unix-only (already true: tokio unix signals, nmcli/iw).
- Non-loopback bind now requires an explicit `--allow-nonlocal-bind` opt-in.
- **Startup-failure asymmetry:** a unix-socket bind failure aborts startup with
  a non-zero exit (it is the mandatory transport), but an opt-in TCP `--bind`
  failure is non-fatal: the daemon logs a loud warning and degrades to serving
  the unix socket only (task-0233), so the optional transport can never take the
  mandatory one down with it. Graceful degrade was chosen over making TCP
  failure fatal, which would let a momentarily-busy opt-in port crash-loop the
  whole daemon into systemd's start-limit under `Restart=on-failure` (task-0204).
- The `whereami` group and the socket's mode/owner are the security-critical
  surface for the system service; the NixOS VM test (task-0209) must assert
  them (socket 0660 `root:whereami`; non-member refused; member served; the
  home-manager 0600 variant; non-loopback bind without the flag refused).
