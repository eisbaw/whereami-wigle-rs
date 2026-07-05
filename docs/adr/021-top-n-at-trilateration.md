# ADR-021: Top-N RSSI applied at trilateration time, not scan time

**Status**: Accepted
**Date**: 2026-05-03

## Context

We only want the N strongest APs for position calculation (weak signals add noise). But if we filter to top-N before the debounce ring, an AP flickering in and out of the top-10 across scans gets inconsistent debounce counts.

## Decision

All observed APs enter the debounce ring buffer unfiltered. The top-N filter is
applied later, at trilateration time: candidates stay ordered by current signal,
but scanner-denied SSIDs, persisted-mobile rows, and non-finite cached positions
are ineligible and cannot consume a slot. The first N eligible cached rows are
used for position calculation. A cold-start fallback may inspect all visible
candidates in the local cache to fill those slots, but remote resolution remains
bounded to the first N scanner-eligible uncached candidates.

## Rationale

- Debounce counts stay accurate. An AP is either stable or not, regardless of whether it's currently the 9th or 11th strongest.
- Signal strength fluctuates scan-to-scan. An AP at -70 dBm might be 10th in one scan and 12th in the next. Debounce shouldn't care.
- API calls are expensive (rate-limited). Cold-start resolution remains capped
  at N uncached scanner-eligible APs even when local cache eligibility requires
  examining more than N rows.

## Consequences

- The debounce ring stores more data (all APs, not just top-N). Negligible memory cost.
- An AP can be "stable" but excluded from trilateration if it is ineligible or
  not in the current top-N eligible set. This is fine — it remains cached for a
  later scan or reclassification on upsert.
