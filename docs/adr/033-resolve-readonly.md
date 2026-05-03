# ADR-033: resolve command doesn't write to cache

**Status**: Accepted
**Date**: 2026-05-03

## Context

The `resolve` command lets clients look up arbitrary BSSIDs. These BSSIDs haven't gone through the debounce filter — they could be transient APs the client saw once.

## Decision

`resolve` queries WiGLE for unknowns but does NOT write results to the `aps` cache. Results are ephemeral — returned to the caller and discarded.

## Rationale

- The debounce filter exists specifically to prevent transient APs from entering the cache. Letting `resolve` bypass it would defeat the purpose.
- A client feeding in a BSSID from a passing bus would permanently cache a position that's only valid for that instant.
- The `locate` path and pending drain are the only writers to `aps` — both go through debounce.

## Consequences

- Repeated `resolve` calls for the same BSSID hit WiGLE each time (no cache benefit). Acceptable for an explicit lookup tool.
- The rate limit counter still increments on `resolve` API calls, so it can exhaust the daily quota if abused.
