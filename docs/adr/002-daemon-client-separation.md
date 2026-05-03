# ADR-002: Daemon + client lib separation

**Status**: Accepted
**Date**: 2026-05-03

## Context

The system needs to scan continuously, maintain a cache, and answer queries. Could be a library, a CLI tool, or a daemon.

## Decision

Long-running daemon (whereamid) owns all state. A thin Rust client library wraps the TCP protocol. Other languages just open a socket directly.

## Rationale

- Daemon accumulates scan history and cache independently of callers. No cold-start penalty per query.
- Background tasks (scanning, pending drain) run regardless of whether anyone is asking.
- Client lib is optional — the protocol is simple enough that `echo '{"cmd":"locate"}' | nc localhost 4747` works.
- Separating the daemon from callers means a Python script, a shell alias, and a Rust binary can all consume the same service.

## Consequences

- Daemon must be running for anything to work. Needs systemd/launchd integration.
- State is process-local (SQLite file). No distributed concerns.
- Client lib is trivially thin — just TCP connect + serde.
