---
id: TASK-0103
title: 'nominatim URL: use reqwest query builder, not format\!() interpolation'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:11'
updated_date: '2026-07-08 18:19'
labels:
  - robustness
  - nominatim
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/nominatim.rs:76-78. URL constructed via format\!() with unescaped lat/lon. For trilaterate-derived floats this is fine. But the function is pub and reachable from any future 'user pastes in coordinates' feature. A NaN/Inf would produce a malformed URL; a manually-crafted lat like '0&radius=1000' would be query-string injection.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Use Client::get(base).query(&[("lat", lat), ("lon", lon), ...]) so reqwest handles escaping
- [x] #2 Unit test: reverse-geocode call with f64::NAN returns Err (not a request to the network)
- [x] #3 Doc-comment: 'lat/lon must be finite; caller is responsible'
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. send_request: replace format!() URL + .get(&url) with client.get(base).query(&[(k, String)...]) so reqwest percent-encodes lat/lon.
2. reverse_geocode_if: add finite guard at the very top (before rate lock + transport dispatch) returning Err for non-finite lat/lon; covers both entry points.
3. Doc-comments on reverse_geocode + reverse_geocode_if: lat/lon must be finite, caller responsible (defence-in-depth).
4. Add unit test mirroring with_transport + AtomicUsize idiom: NAN lat and INF lon both Err, transport counter stays 0.
5. Gate: cargo test nominatim (x2), just e2e, clippy -D warnings, fmt --check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented (commit 2f02994):
- send_request now uses client.get(base).query(&[(&str, String)...]) — the String value shape compiled cleanly; reqwest percent-encodes.
- Finite guard placed in reverse_geocode_if (not send_request/reverse_geocode) so it covers BOTH public entry points and fires BEFORE the rate lock + (self.transport)(...) dispatch. Confirmed via test that transport counter stays 0.
- Doc-comments added to both reverse_geocode and reverse_geocode_if stating lat/lon must be finite (caller responsible; guard is defence-in-depth).
- New test non_finite_coordinates_reject_without_transport: NAN lat -> Err, INF lon -> Err, AtomicUsize transport counter == 0 in both cases (proves no network request). Asserts on counter/Result, not logs.

Gate (all via nix develop -c):
- cargo test nominatim: 2 passed 0 failed (ran twice, both green)
- just e2e: full suite green (unit 12 passed, shutdown 4 passed, doc-tests 0)
- clippy --all-targets -D warnings: clean
- cargo fmt --check: clean

Left In Progress for review gate; ACs not checked by implementer.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Hardened the Nominatim reverse-geocode request path (commit 2f02994, whereamid/src/nominatim.rs).

Changes:
- send_request builds the URL via reqwest .get(base).query(&[("lat",..),("lon",..),("format","json"),("addressdetails","1")]) instead of format!() interpolation (AC#1). Semantically identical for finite f64 (same params, same order, same Display rendering — byte-identical URL for legitimate input); reqwest percent-encodes, so this is future-proofing for any later string-sourced coordinate feature.
- reverse_geocode_if rejects non-finite lat/lon at the top, BEFORE the rate-gate lock and the transport dispatch (AC#2), so a NaN/Inf returns Err with no network request and no rate slot consumed. Both public entry points (reverse_geocode, reverse_geocode_if) funnel through this choke point; the sole production caller (server/geocode.rs background worker) already handles Err as warn+continue, so no spurious error can reach a locate response.
- Doc-comments on both entry points state the finite requirement as defence-in-depth (fail-closed), not a contract callers may lean on (AC#3).
- Test non_finite_coordinates_reject_without_transport: mock transport + AtomicUsize counter; NaN-lat and Inf-lon each return Err with the counter still 0 (proving no network dispatch). Bites the old code (which would have called the transport and returned Ok).

HONEST SCOPE CORRECTION (from mped-architect review): the task text cited a query-string-INJECTION vector (a lat like "0&radius=1000"), but that is unreachable with the current f64 signature — f64::to_string() cannot emit query delimiters, so the old format!() was already injection-safe for f64. The actual live defect fixed here is the MALFORMED-URL case (a non-finite value would embed the literal token NaN/inf), closed by the finite guard. The .query() migration is legitimate hygiene/future-proofing, NOT a patch of a live injection hole. Coverage note: AC#1 (query builder) is not independently unit-tested because the with_transport test seam replaces send_request; verifying reqwest URL assembly is low value.

Verification: review qa GO (741 workspace tests, whereamid bin 450, clippy -D warnings clean, fmt clean, new test passes twice) / mped GO (query migration semantically equivalent, guard placement + fail-closed direction + caller interaction all confirmed correct, test bites, docs honest).
<!-- SECTION:FINAL_SUMMARY:END -->
