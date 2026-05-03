# ADR-030: Cache-first, query on miss

**Status**: Accepted
**Date**: 2026-05-03

## Context

WiGLE API has daily rate limits. Querying every AP on every locate would exhaust the quota in minutes.

## Decision

On `locate`: check SQLite first for each stable BSSID. Only query WiGLE for cache misses. AP positions rarely change, so cached data stays valid indefinitely.

## Rationale

- AP hardware positions are essentially static (routers don't move daily).
- Once the cache is warm for your area, `locate` works with zero network traffic — fully offline.
- Rate limit is a hard constraint. Cache-first minimizes API calls to the absolute minimum: one call per unique BSSID, ever.

## Consequences

- First locate in a new area is slow (multiple API calls). Subsequent ones are instant.
- Stale cache for moved APs. Mitigated by the outlier filter (ADR-022), not by cache expiry.
- No TTL on cached positions by default. A `purge` command could be added if needed.
