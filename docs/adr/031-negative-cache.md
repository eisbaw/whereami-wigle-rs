# ADR-031: Negative cache (not_found table)

**Status**: Accepted
**Date**: 2026-05-03

## Context

Many home routers have never been wardrive-scanned. WiGLE returns 404 for them. Without remembering this, the daemon re-queries the same unknown BSSID on every locate, burning rate limit for nothing.

## Decision

Store 404 results in a `not_found` table. Check it before querying WiGLE. Re-check after a configurable TTL (default 30 days, `--not-found-ttl-days`).

## Rationale

- In testing, 2 of 11 visible APs were unknown to WiGLE. Without negative cache, that's 2 wasted API calls per locate cycle.
- 30-day TTL allows re-checking in case someone wardrive-scans the area and uploads to WiGLE.
- Cheap to store (just BSSID + timestamp). No position data.

## Consequences

- A genuinely new AP that gets added to WiGLE won't be picked up for 30 days. Acceptable tradeoff vs burning rate limit.
- The pending drain task also re-checks expired not_found entries, so recovery is automatic.
