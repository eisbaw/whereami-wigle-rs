# ADR-030: Cache-first, query on miss

**Status**: Accepted
**Date**: 2026-05-03
**Amended**: 2026-07-05 (bounded retention and explicit privacy wipe)

## Context

WiGLE API has daily rate limits. Querying every AP on every locate would exhaust the quota in minutes.

## Decision

On `locate`: check SQLite first for each stable BSSID. Only query providers for
cache misses. A cache hit refreshes `last_seen`. By default a background sweep
removes AP rows not observed for 30 days; `--aps-retention-days 0` restores
indefinite retention.

Expose an explicitly confirmed `wipe` command on the access-controlled Unix
socket. It deletes all logical AP/location rows atomically and clears managed
in-memory location state. Every socket-authorized user can invoke it; there is
no read-only or per-command role, and confirmation is not authorization. The
unauthenticated TCP transport must refuse it.

## Rationale

- AP hardware positions are essentially static (routers don't move daily).
- Once the cache is warm for your area, `locate` works with zero network traffic — fully offline.
- Rate limit is a hard constraint. Cache-first minimizes API calls while an AP
  remains actively observed.
- A bounded default reduces long-lived BSSID/SSID/location traces without
  making cache expiry part of the latency-sensitive locate path.
- A user on a shared or at-risk machine needs a supported immediate deletion
  boundary rather than manual SQLite surgery.

## Consequences

- First locate in a new area is slow (multiple API calls). Subsequent ones are instant.
- Stale cache for moved APs. Mitigated by the outlier filter (ADR-022), not by cache expiry.
- APs unseen for the configured window are re-fetched if encountered later.
  Setting the window to 0 preserves the original indefinite-cache behaviour.
- `whereami wipe --yes` deletes logical live state, but collection remains
  enabled and may repopulate after the response-write attempt terminates by
  success, failure, timeout, or disconnect. The operation commits SQLite,
  clears managed memory while exclusive, and advances its publication epoch
  last. A client without a complete response must verify or retry idempotently.
- A panic after wipe commit retains the publication guard through a nonzero
  fail-stop exit; recovering only the database actor could expose empty
  persisted state alongside stale memory.
- Wipe is not forensic erasure of SQLite free pages/WAL remnants,
  allocator/process-memory remnants outside managed caches, snapshots, backups,
  logs, storage media, or provider traffic already sent.
