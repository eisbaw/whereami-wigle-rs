# ADR-021: Top-N RSSI applied at trilateration time, not scan time

**Status**: Accepted
**Date**: 2026-05-03

## Context

We only want the N strongest APs for position calculation (weak signals add noise). But if we filter to top-N before the debounce ring, an AP flickering in and out of the top-10 across scans gets inconsistent debounce counts.

## Decision

All observed APs enter the debounce ring buffer unfiltered. The top-N filter is applied later, at trilateration time: of the stable APs, only the N strongest (by signal in the most recent scan) are used for position calculation and API lookups.

## Rationale

- Debounce counts stay accurate. An AP is either stable or not, regardless of whether it's currently the 9th or 11th strongest.
- Signal strength fluctuates scan-to-scan. An AP at -70 dBm might be 10th in one scan and 12th in the next. Debounce shouldn't care.
- API calls are expensive (rate-limited). Only spending them on the top-N stable APs minimizes waste.

## Consequences

- The debounce ring stores more data (all APs, not just top-N). Negligible memory cost.
- An AP can be "stable" but excluded from trilateration if it's not in the current top-N. This is fine — it's still cached for next time.
