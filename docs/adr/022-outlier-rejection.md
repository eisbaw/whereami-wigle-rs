# ADR-022: Outlier rejection via adaptive median

**Status**: Accepted
**Date**: 2026-05-03

## Context

WiGLE data can be stale. A router that physically moved still has its old position in the database. A naive weighted centroid gets dragged toward these outliers — in testing, one AP from Brazil and one from 2.5km away corrupted the result entirely.

## Decision

Before trilateration, filter outliers using the median:
1. Compute median lat/lon of all APs (robust center estimate)
2. Compute each AP's distance from that median
3. Compute the median of those distances (typical spread)
4. Reject APs whose distance from median exceeds `max(200m, 3 × median_distance)`

## Rationale

- **200m floor**: physical assumption that legitimate APs are within ~200m of each other in a neighborhood.
- **3× median adapts**: in sparse areas where all APs are 300m apart, threshold becomes 900m — nothing gets rejected. No hard cutoff that could reject everything.
- **Median, not mean**: a single outlier 5000km away doesn't shift the median. This is the key property.
- Iterating not needed — single pass with the adaptive threshold handles all observed cases (Brazil outlier, moved router 2km away, tight clusters).

## Consequences

- Routers that moved are silently excluded. No feedback to the user about which APs were dropped (could add to response if needed).
- In pathological cases (exactly 2 APs far apart), both are kept since we skip filtering for ≤2 APs. Single-AP accuracy applies.
- The 200m assumption breaks in rural areas with distant APs. The 3× median handles this, but if ALL APs have stale positions in different directions, the median itself is wrong. Unlikely in practice.
