# ADR-060: Reverse geocoding via OSM Nominatim, opt-in

**Status**: Accepted (amended)
**Date**: 2026-05-03
**Amended**: 2026-07-05 (TASK-0244)

## Context

Latitude/longitude coordinates are precise but not human-readable. Users may
want an approximate street address. Candidate services included Google
Geocoding (paid and authenticated), the public OpenStreetMap Nominatim service,
and self-hosted systems such as Nominatim, Photon, or Pelias.

Reverse geocoding also changes the privacy and reliability profile of `locate`:
an external service receives a position, and its latency or availability must
not decide whether the underlying Wi-Fi location succeeds.

## Original decision (2026-05-03)

Use OpenStreetMap Nominatim for reverse geocoding, opt-in through
`--address-approx`. The public endpoint required no project API credential, and
the request used a meaningful User-Agent. The `address` field was deliberately
named without an "exact" qualifier because both the Wi-Fi fix and the returned
address are approximate.

The initial execution model put the Nominatim lookup directly on an enabled
`locate` response path and did not yet reuse resolved addresses. A lookup
failure was non-fatal, but the request latency was still paid by that `locate`.
Those execution details are superseded by the amendment below; the provider,
opt-in policy, and approximate field semantics remain accepted.

## Amendment (2026-07-05): asynchronous, cached, and bounded

The current design evolved in four steps: TASK-0025 moved Nominatim's HTTP and
rate wait off the response path and introduced the initial process-local cache
bounded at 256 cells; TASK-0037 made its TTL configurable (default 7 days) while
preserving that cap; TASK-0046 made matching `last_fix` backfill safe across its
durable write; and TASK-0193 replaced per-miss tasks with one latest-only
worker. TASK-0244 records that implementation as the current decision.

### Locate response path

For a newly computed fix:

1. With `--address-approx` disabled, the fresh-fix path skips both the address
   cache and scheduler. It updates the in-memory fix without an address and
   attempts the durable fix/history write.
2. With the flag enabled, the daemon checks an in-memory cache keyed by the fix's
   latitude/longitude rounded to four decimal places (roughly an 11 m latitude
   grid at the equator).
3. A cache hit can populate `address` in the current response without initiating
   or awaiting a new Nominatim request for that locate. The fresh in-memory fix
   includes the address, its durable fix/history write is attempted, and the hit
   supersedes older geocode work that is no longer relevant.
4. A miss or expired entry leaves `address` omitted from the current response.
   The daemon updates the in-memory fix, attempts its durable fix/history write,
   and publishes the miss observation to the bounded scheduler. Observation is
   not a guarantee that a new candidate or outbound call will result: same-cell
   deduplication, supersession, shutdown, or queue age can prevent it.

Publishing work is not promised to happen after the client receives its
response: the named worker can wake while the durable write or socket response
is still in progress. The contract is that `locate` never awaits Nominatim's
one-second rate gate or HTTP request.

If an eligible background request later succeeds while its candidate is still
current, the worker inserts the address into the process-local cache, updates
the matching in-memory `last_fix`, and attempts the guarded durable backfill. A
later eligible locate in that cell may then return the cached address; the next
locate is not guaranteed to do so. Failed, unreachable, timed-out, or empty
requests and superseded work leave the address unavailable and do not turn the
coordinate-success response into an error. There is no autonomous retry loop;
a later eligible cache miss may publish work again.

### Bounded scheduling and outbound contact

- One named worker owns reverse-geocode rate waiting and HTTP work. Locate calls
  do not spawn geocode tasks.
- Scheduler state permits at most one active candidate plus one pending slot.
  The pending slot keeps only the latest distinct candidate; intermediate
  positions are overwritten.
- The rounded cell is the exact-key deduplication identity while that cell
  remains latest. Repeated latest observations of a pending or active cell
  coalesce, but a sequence A → B → A makes the returning A a new generation
  and therefore new eligible work. If the original A was already dispatched,
  it may already have reached Nominatim.
- Generation and queue-age checks reject superseded or stale work before the
  transport boundary. A newer observation can also cancel local processing of
  an older in-flight future, and stale results cannot mutate the cache or
  `last_fix`.
- The Nominatim client enforces at least one second between outbound dispatches
  from this daemon process and rechecks candidate freshness after waiting at the
  gate. A rejected candidate does not consume a rate slot.
- Queue age applies only before dispatch. Its current budget is one one-second
  rate interval plus one second of scheduling tolerance (two seconds total).
  Once an unsuperseded request has been dispatched, queue age does not expire
  its response; it may run until completion or the configured HTTP timeout.
- Each HTTP attempt has the total timeout configured by
  `--nominatim-timeout-secs` (default 30). That timeout occupies the background
  worker, not the locate response path, unless newer work or shutdown supersedes
  the local future first.

These bounds reduce redundant contact; they do not promise one request for every
locate, nor do they promise that every miss reaches Nominatim. The limiter is
per daemon process and does not coordinate multiple daemon instances or other
Nominatim clients.

The four-decimal rounded cell is only a local cache and deduplication key.
Candidates retain the actual computed latitude/longitude, and the selected
outbound Nominatim request sends those actual coordinates. Rounding is not a
privacy anonymization mechanism. Likewise, supersession can prevent an older
candidate from being sent or reject its result, but it cannot retract a request
that the remote service has already received.

### Cache and persistence lifecycle

- The address cache is process-local memory, not a persisted cache. It holds at
  most 256 distinct rounded cells and evicts the oldest inserted cell when the
  bound is exceeded; reads do not promote entries, so this is not an LRU cache.
- Entries at or beyond `--address-cache-ttl-days` (default 7) are treated as
  misses on read, not eagerly deleted. A value of 0 disables cache hits, not
  reverse geocoding: with `--address-approx` still enabled, misses may continue
  to publish background observations.
- Fresh hit and miss paths update the in-memory fix and attempt durable
  fix/history writes. These writes are best-effort: a database failure is
  counted and warned but does not revoke the coordinate response.
- A successful current worker result inserts the process-local cache first,
  then updates the matching in-memory `last_fix` and attempts its guarded
  durable backfill. The cache entry survives a backfill failure; the cache
  itself is never persisted.
- On daemon restart only a successfully persisted `last_fix.address` may
  survive; the cache starts empty. A stale fallback reads that saved address
  directly rather than consulting the address cache.
- The named worker exists independently of the flag. With
  `--address-approx` disabled, the fresh path skips cache observation and
  publication, so the worker has no new geocode work and remains idle. If
  current trilateration fails, a stale persisted last fix can still return its
  saved `address` regardless of the current flag. Returning that saved string
  causes no new Nominatim contact.

## Rationale for the amendment

- `locate` never awaits Nominatim HTTP or its one-second gate, so network latency
  and the configured Nominatim timeout stay off that response path.
- A cache hit issues no new outbound request for that locate and consumes no rate
  slot, while TTL expiry allows address data to refresh.
- A single worker and bounded state prevent movement or a burst of locates from
  creating an unbounded task or request queue.
- Latest-only supersession favors the position that is still useful without
  claiming that already-issued network traffic can be undone.
- Opt-in remains necessary: a cache miss can disclose the actual computed
  position to a third-party service. Users requiring no new reverse-geocode
  traffic must leave `--address-approx` disabled.

## Consequences

- `address` is opportunistic. Clients must accept its omission on fresh fixes,
  including the first locate in a cell, after TTL expiry, after restart, or when
  Nominatim fails.
- `locate` never awaits Nominatim HTTP or the one-second rate gate, and a
  Nominatim network failure cannot turn a coordinate-success response into an
  error. This does not remove response-path latency from local locks or the
  best-effort durable fix write. Background failures are logged; only a later
  eligible miss can publish another attempt.
- The one-second per-process gate, deduplication, latest-only pending slot, and
  bounded cache constrain outbound traffic, but are not a blanket guarantee of
  compliance for multiple processes or unrelated clients. Operators remain
  responsible for the service's usage policy.
- The default remains privacy-preserving with respect to new reverse-geocode
  traffic because the feature is off unless explicitly enabled. Enabling it
  permits actual coordinates to leave the machine on cache misses.
- Self-hosting Nominatim or another compatible geocoder remains a possible
  future direction for deployments that need local address resolution.

## Amendment history

- **2026-05-03** — Original provider and opt-in decision accepted, with a
  synchronous, uncached execution model.
- **2026-05-10, TASK-0025** — Split Nominatim HTTP/rate waiting from the
  response and added the initial rounded-cell cache with a 256-cell bound.
- **2026-05-10, TASK-0037** — Added the configurable cache TTL (default 7 days)
  while preserving the 256-cell cap.
- **2026-05-10, TASK-0046** — Made matching `last_fix` backfill retain safe
  in-memory/durable ordering.
- **2026-07-05, TASK-0193** — Replaced per-miss tasks with the single-worker,
  one-active/latest-pending scheduler and dispatch-time supersession checks.
- **2026-07-05, TASK-0244** — Amended the ADR to make the already-implemented
  cache, worker, persistence, and privacy semantics authoritative; the original
  synchronous execution details are no longer current.
