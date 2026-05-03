# ADR-032: Pending queue for offline resilience

**Status**: Accepted
**Date**: 2026-05-03

## Context

The daemon may be offline (airplane mode, no internet), or WiGLE may rate-limit (429). BSSIDs that need lookup shouldn't be lost.

## Decision

Unresolvable BSSIDs go into a `pending` SQLite table. A background task drains it every N seconds (default 300s) when WiGLE becomes reachable. Max attempts before giving up (default 20).

## Rationale

- SQLite persistence means pending BSSIDs survive daemon restarts. Nothing is lost.
- Background drain is independent of client requests. No one needs to call `locate` for the queue to process.
- Max attempts prevents permanent retry loops for genuinely broken cases (persistent network errors to a specific MAC query).
- 429 from WiGLE immediately stops the drain run — respects rate limits.

## Consequences

- `locate` in a new area with no internet returns "no APs with known positions." Correct — it can't resolve anything.
- Once connectivity returns, the pending queue catches up automatically. Next `locate` finds everything cached.
- Pending table grows linearly with unique unknown APs. Bounded by max_attempts cleanup.
