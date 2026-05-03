# ADR-040: WAL mode + schema versioning

**Status**: Accepted
**Date**: 2026-05-03

## Context

The daemon has multiple concurrent async tasks (TCP handlers, background scanner, pending drain) all accessing the same SQLite database. Default SQLite journal mode serializes writes and can return "database is locked."

## Decision

Enable WAL mode on database open. Track schema version in a dedicated table for future migrations.

## Rationale

- WAL (Write-Ahead Logging) allows concurrent readers alongside a single writer without blocking. Our workload is read-heavy (cache lookups on every locate) with infrequent writes.
- Schema versioning now costs one row but saves painful ad-hoc migration logic later. First schema change will be trivial to implement.
- Verify WAL actually took effect (some filesystems don't support it). Warn if not.

## Consequences

- WAL creates `-wal` and `-shm` files alongside the database. Must be on same filesystem.
- Slightly more disk I/O than rollback journal, but dramatically better concurrency.
- Schema migrations are the daemon's responsibility on startup. No external migration tool.
