# ADR-020: Debounce before commit

**Status**: Accepted
**Date**: 2026-05-03

## Context

A single Wi-Fi scan sees everything in radio range: your neighbors' routers (stable), but also phones being hotspots, buses passing by, and construction site radios (transient). Committing all of them to the cache and querying WiGLE for each would fill the database with garbage positions.

## Decision

A BSSID must appear in at least M of the last N scan samples before it is considered "stable." Only stable BSSIDs are looked up via API, committed to SQLite, or used for trilateration.

Defaults: N=10 samples, M=5 hits. Configurable via `--debounce-window` and `--debounce-threshold`.

## Rationale

- A bus passing takes ~10-30 seconds. With 10s scan intervals, it appears in 1-3 samples — below the threshold of 5.
- A fixed router appears in every scan — easily hits threshold.
- Sliding window (not cumulative counter) means APs that leave also get "unstable" and stop being used.
- All in-memory (VecDeque). Lost on restart, which is fine — the daemon just needs a minute to re-learn.

## Consequences

- Transient APs never waste API calls or pollute the cache.
- Raw `scan` command still shows everything unfiltered (for debugging).
- Moving in a vehicle means no AP is stable — `locate` returns an error until you stop. This is correct behavior.
