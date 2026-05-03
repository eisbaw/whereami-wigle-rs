# ADR-060: Reverse geocoding via OSM Nominatim, opt-in

**Status**: Accepted
**Date**: 2026-05-03

## Context

Lat/lon coordinates are precise but not human-readable. Users want a street address. Options: Google Geocoding API (paid, API key), Nominatim (free, no auth), self-hosted Photon/Pelias.

## Decision

Use OpenStreetMap Nominatim for reverse geocoding. Opt-in via `--address-approx` flag. When disabled, no geocoding call is made and the field is absent from the response.

## Rationale

- No auth, no API key, no cost. Just a User-Agent header and respect for rate limits (1 req/sec).
- OSM data quality in Europe is excellent — often better than commercial alternatives for street-level detail.
- Opt-in because it adds latency (~100-300ms per locate) and leaks your position to a third-party server. Users who want fully offline operation shouldn't pay this cost.
- The field name is `address` (not `address_exact`) — the word "approx" is in the flag name to set expectations. Wi-Fi geolocation is inherently approximate.

## Consequences

- Adds a network call to the locate path when enabled. If Nominatim is down, locate still succeeds (address is None, warning logged).
- Nominatim usage policy: max 1 req/sec, meaningful User-Agent, no bulk use. Our usage (one call per locate, not continuous) is well within bounds.
- No caching of geocode results. The same lat/lon could be re-geocoded on each locate. Acceptable since locate frequency is low and the positions shift slightly each time.
- Future: could self-host Nominatim or Photon for true offline reverse geocoding.
