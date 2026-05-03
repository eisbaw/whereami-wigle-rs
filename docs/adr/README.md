# Architecture Decision Records

## Communication
- [ADR-001](001-tcp-json-lines.md) — TCP JSON-lines, not HTTP
- [ADR-002](002-daemon-client-separation.md) — Daemon + client lib separation

## Scanning
- [ADR-010](010-nmcli-over-iw.md) — nmcli as primary scanner, iw as fallback
- [ADR-011](011-scan-backoff.md) — Scan backoff: fast then slow

## Filtering
- [ADR-020](020-debounce-before-commit.md) — Debounce before commit
- [ADR-021](021-top-n-at-trilateration.md) — Top-N RSSI at trilateration time, not scan time
- [ADR-022](022-outlier-rejection.md) — Outlier rejection via adaptive median

## Caching & API
- [ADR-030](030-cache-first.md) — Cache-first, query on miss
- [ADR-031](031-negative-cache.md) — Negative cache (not_found table)
- [ADR-032](032-pending-queue.md) — Pending queue for offline resilience
- [ADR-033](033-resolve-readonly.md) — resolve command doesn't write to cache

## Storage
- [ADR-040](040-wal-and-schema-versioning.md) — WAL mode + schema versioning
- [ADR-041](041-cli-args-toml-secrets.md) — CLI args for operations, TOML for secrets

## Deployment
- [ADR-050](050-nixos-native.md) — NixOS-native with systemd hardening
