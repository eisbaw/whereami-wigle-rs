# whereami — Local Wi-Fi Geolocation Service

## Problem

Browser-based geolocation (Google, Mozilla) requires sending your Wi-Fi environment to third-party servers on every query. Offline geolocation on Linux doesn't exist in usable form. GeoClue's Mozilla backend is dead, and BeaconDB doesn't yet offer data exports.

We want: scan local Wi-Fi, resolve MAC addresses to coordinates, cache results locally, and answer "where am I?" without phoning home once the cache is warm.

## Architecture

Two components:

```
                    TCP :4747
  any client  ------>  whereamid  ----> local SQLite cache
  (CLI, curl,            |                    |
   Python, etc.)         |              cache miss?
                         |                    |
                         +-----------> Apple WPS  (remote, no auth, primary)
                                       WiGLE API  (remote, secondary)
```

The Source enum still recognises `beacondb` so historical DB rows from
earlier prototypes read back correctly at priority 20, but no BeaconDB
client runs today (the API returns aggregate position, not per-AP, so
it does not fit the per-BSSID Provider chain).

### whereamid (daemon)

Long-running process. Listens on a TCP socket (default `127.0.0.1:4747`). Newline-delimited JSON protocol (one JSON object per line, one JSON response per line). No HTTP overhead — just raw TCP + JSON lines.

Responsibilities:
- Accept queries over TCP
- Manage the local SQLite cache (read/write)
- Perform Wi-Fi scans (via `iw dev wlan0 scan` or nl80211)
- Call remote APIs on cache miss (Apple WPS first, then WiGLE)
- Trilaterate position from known AP locations
- Rate-limit outbound API calls to stay within WiGLE daily limits

### whereami-client (Rust library crate)

Thin TCP client. Connect, send JSON, read JSON response. Exists so Rust callers don't hand-roll the protocol. Other languages just open a TCP socket and send/receive JSON lines — no client library needed.

## TCP Protocol

Request and response are each a single line of JSON terminated by `\n`.

**Connection lifecycle**: one-shot. Client connects, sends one JSON line, reads one JSON response line, connection closes. No persistent connections, no request multiplexing. Localhost TCP setup is sub-millisecond so the overhead is negligible.

All responses include a `"v": 1` field for protocol versioning.

### Commands

**`locate`** — "Where am I right now?"

Cache-only hot path: takes the current stable APs (debounce filter), trilaterates from whatever positions are already in the local `aps` cache, and responds immediately. Remote provider calls happen in background tasks (scan loop, pending drain, cold-start fallback) — `locate` itself never awaits an external API. If no stable APs have cached positions, the response is either an error or, if available, the previous known fix flagged with `stale: true` and `age_s`.

If `--address-approx` is enabled, an approximate street address is attached when the rounded position is in the local address cache; on a miss the address resolves in the background and the next locate at the same rounded position will include it.

```json
{"cmd": "locate"}
```

Response:
```json
{"ok": true, "v": 1, "lat": 55.6684, "lon": 12.5541, "accuracy_m": 30, "sources": 7, "cached": 5, "fetched": 2, "pending": 1, "visible": 23, "stable": 7}
```

`visible` = total APs seen in latest scan.
`stable` = APs passing the debounce filter (seen in >= 5 of last 10 samples).
`sources` = stable APs with known positions (contributed to trilateration).
`cached` / `fetched` = how many came from local cache vs remote API call just now.
`pending` = MACs queued for later lookup (WiGLE unreachable or quota exhausted).
`accuracy_m` = estimated accuracy in meters (derived from AP spread + signal variance).

**`resolve`** — Look up specific BSSIDs. Read-only against the cache: returns what is known locally, queries WiGLE for misses, but does **not** write results to the `aps` cache (since these BSSIDs have not been debounce-verified as stable). Results are returned but ephemeral.

```json
{"cmd": "resolve", "bssids": ["AA:BB:CC:DD:EE:FF", "11:22:33:44:55:66"]}
```

Response:
```json
{"ok": true, "v": 1, "results": [
  {"bssid": "AA:BB:CC:DD:EE:FF", "lat": 55.668, "lon": 12.554, "ssid": "Cafe-WiFi", "source": "cache"},
  {"bssid": "11:22:33:44:55:66", "lat": null, "lon": null, "ssid": null, "source": "not_found"}
]}
```

**`scan`** — Return visible Wi-Fi networks without resolving locations.

```json
{"cmd": "scan"}
```

Response:
```json
{"ok": true, "v": 1, "networks": [
  {"bssid": "AA:BB:CC:DD:EE:FF", "ssid": "Cafe-WiFi", "signal_dbm": -65, "channel": 6},
  {"bssid": "11:22:33:44:55:66", "ssid": "eduroam", "signal_dbm": -78, "channel": 36}
]}
```

**`stats`** — Cache and pending queue statistics.

```json
{"cmd": "stats"}
```

Response:
```json
{"ok": true, "v": 1, "cached_aps": 14823, "pending_aps": 3, "not_found_aps": 12, "db_size_bytes": 2097152, "api_calls_today": 42}
```

### Error shape

```json
{"ok": false, "error": "wifi scan failed: permission denied"}
```

## Data Model (SQLite)

Two tables in the same database file:

All timestamps are **UTC**. SQLite in **WAL mode** (set on open via `PRAGMA journal_mode=WAL`).

```sql
-- Schema version tracking
CREATE TABLE schema_version (
    version     INTEGER NOT NULL
);
INSERT INTO schema_version (version) VALUES (1);

-- Resolved APs with known positions
CREATE TABLE aps (
    bssid       TEXT PRIMARY KEY,  -- normalized uppercase, colon-separated
    ssid        TEXT,
    lat         REAL NOT NULL,
    lon         REAL NOT NULL,
    encryption  TEXT,              -- informational, from WiGLE (may differ from scan)
    channel     INTEGER,           -- informational, from WiGLE
    frequency   INTEGER,           -- informational, MHz, from WiGLE
    city        TEXT,              -- informational, from WiGLE
    country     TEXT,              -- informational, from WiGLE
    source      TEXT NOT NULL,     -- "apple" | "wigle" | "beacondb" | "manual"
    first_seen  TEXT NOT NULL,     -- UTC ISO 8601, when daemon first observed this AP
    last_seen   TEXT NOT NULL,     -- UTC ISO 8601, most recent scan that saw it
    fetched_at  TEXT NOT NULL      -- UTC ISO 8601, when position was resolved via API
);

CREATE INDEX idx_aps_geo ON aps(lat, lon);
CREATE INDEX idx_aps_last_seen ON aps(last_seen);

-- Negative cache: BSSIDs that WiGLE confirmed do not exist (404).
-- Prevents repeated futile lookups for unknown MACs.
CREATE TABLE not_found (
    bssid       TEXT PRIMARY KEY,  -- normalized uppercase, colon-separated
    first_seen  TEXT NOT NULL,     -- UTC ISO 8601
    last_seen   TEXT NOT NULL,     -- UTC ISO 8601
    checked_at  TEXT NOT NULL      -- UTC ISO 8601, when WiGLE returned 404
);

-- Pending BSSIDs: stable APs we've seen but couldn't resolve yet
-- (WiGLE unreachable, quota exhausted, etc.)
CREATE TABLE pending (
    bssid       TEXT PRIMARY KEY,  -- normalized uppercase, colon-separated
    ssid        TEXT,
    channel     INTEGER,
    frequency   INTEGER,           -- MHz
    signal_dbm  INTEGER,           -- strongest observed signal
    first_seen  TEXT NOT NULL,     -- UTC ISO 8601, when first queued
    last_seen   TEXT NOT NULL,     -- UTC ISO 8601, most recent scan that saw it
    attempts    INTEGER NOT NULL DEFAULT 0
);

-- API call tracking for daily rate limiting
CREATE TABLE metadata (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL
);
-- Tracks: "api_calls_today" (integer), "api_calls_date" (UTC date YYYY-MM-DD)
-- Reset logic: if api_calls_date != today (UTC), reset api_calls_today to 0.
```

The **pending table** acts as a work queue. The daemon drains it in the background whenever WiGLE becomes reachable:
- On successful lookup: row moves from `pending` to `aps`, deleted from `pending`
- On WiGLE 404 (MAC genuinely unknown): row moved to `not_found`, deleted from `pending`
- On network error / 429: `attempts` incremented, row stays in `pending`
- On `attempts >= max_attempts` (default 20, configurable via `--pending-max-attempts`): row deleted from `pending`, treated as permanently unresolvable

No expiry on `aps` by default. APs don't move often. A future `purge` command could remove entries older than N days. Entries in `not_found` can be re-checked after a configurable TTL (default 30 days).

## Wi-Fi Scanning

Two approaches, in preference order:

1. **nl80211 via netlink** — direct kernel interface, no subprocess. Crate: `neli` or `genetlink`. Requires `CAP_NET_ADMIN`.
2. **Shell out to `iw dev <iface> scan`** — simpler, same capability requirement. Parse the text output.

Start with option 2. Move to option 1 if parsing `iw` output proves fragile.

**Important**: `iw dev wlan0 scan` fails with "Device or resource busy" when the interface is associated to an AP (which it normally will be). Use the two-step approach: `iw dev wlan0 scan trigger` followed by `iw dev wlan0 scan dump`. The trigger initiates a background scan; the dump reads cached results. This works while connected.

The daemon needs to run as root or with `CAP_NET_ADMIN` for scanning. Alternatively, accept scan results from clients (the `resolve` command) so the daemon itself doesn't need elevated privileges.

### Scan filtering and debounce interaction

Each scan records **all** observed APs into the debounce ring buffer (not just top-N). This ensures debounce counts are accurate regardless of RSSI fluctuation between scans. The **top-N RSSI filter** is applied later, at trilateration time: of the stable APs, only the N strongest (from the most recent scan's signal readings) are used for position calculation and API lookups.

### Debounce: stable AP detection

Transient APs (phones as hotspots, buses, trains) pollute the cache with positions that are only valid for seconds. To prevent this, a MAC must prove it is stationary before being committed to SQLite or looked up on WiGLE.

**Sliding window**: the daemon maintains a ring buffer of the last N scan samples. A MAC is considered **stable** when it appears in at least M of those N samples.

Defaults: N=10 samples, M=5 hits required. Configurable via CLI args `--debounce-window` and `--debounce-threshold`.

Only stable MACs are:
- Looked up via remote APIs (or queued to pending if offline)
- Written to the SQLite cache (via pending -> resolved flow)
- Used for trilateration in `locate` responses

Unstable MACs are silently ignored. They still appear in raw `scan` responses (which report what the radio sees, unfiltered).

**In-memory state** (not persisted):

```
scan_ring: VecDeque<ScanSample>    // last N samples
                                    // each sample: HashMap<MAC, signal_dbm>

fn is_stable(mac) -> bool {
    scan_ring.iter().filter(|s| s.contains(mac)).count() >= M
}
```

This state is ephemeral — lost on daemon restart, which is fine. The daemon needs a few scan cycles before it can distinguish stable from transient APs.

**Cold-start behaviour**: with defaults (N=10, M=5, fast scan every 10s), it takes ~50 seconds before any MAC can be "stable." During this window, `locate` will still work if it finds the scanned BSSIDs already in the `aps` cache (from a previous run). Only genuinely new, never-seen BSSIDs must wait for debounce. This means restarts in a known location are near-instant; only travel to a new location has the 50s warm-up.

### Continuous background scanning

The daemon scans continuously in the background, independent of client requests. Scan interval uses exponential backoff:

- **Fast phase**: every `--scan-interval-fast` seconds (default: 10s) for the first `--scan-fast-duration` seconds after start (default: 60s)
- **Steady phase**: every `--scan-interval-slow` seconds (default: 60s) thereafter

This ensures the debounce ring buffer fills quickly on startup, then tapers off to reduce radio and CPU load. A `locate` request never triggers a scan itself — it reads whatever the background scanner has accumulated.

### Pending queue drain

A separate background task periodically attempts to resolve MACs in the `pending` table:

- Runs every `--pending-interval` seconds (default: 300s / 5 minutes)
- On each run: pick up to 10 pending MACs, query WiGLE
- On success: move to `aps` table, delete from `pending`
- On network error: increment `attempts`, leave in `pending`
- On WiGLE 404 (unknown MAC): delete from `pending`
- If WiGLE returns 429: stop the drain run, retry next interval

## Remote API Backends

### Apple WPS (primary)

- Endpoint: Apple's location services protobuf endpoint (no public docs; reverse-engineered from iOS).
- Auth: none.
- Returns coordinates for the queried BSSID plus up to ~400 nearby BSSIDs in a single response, all stored in the local cache.
- Quotas: no documented daily cap. The daemon does not rate-limit it; we treat it as best-effort.

### WiGLE (secondary)

- Endpoint: `GET https://api.wigle.net/api/v2/network/search?netid=<MAC>`
- Auth: HTTP Basic (configured in daemon config).
- Rate limit: respect daily quota. The daemon reserves and refunds slots in the SQLite `metadata` table under a single transaction (read+increment in one atomic step).
- Returns: `trilat`, `trilong`, `ssid`, `encryption`, `city`, `country`

### BeaconDB (currently unused)

A BeaconDB client exists in the codebase but is not wired into the active resolution chain. It was retained pending an exports feature; in the meantime the resolver never calls it. Treat the section as documentation of intent rather than runtime behaviour.

- Endpoint: `POST https://beacondb.net/v1/geolocate`
- Body: `{"wifiAccessPoints": [{"macAddress": "AA:BB:CC:DD:EE:FF"}]}`
- No auth required
- Limited database (~108M networks)

### Lookup strategy (on `locate` request)

`locate` is cache-only on the hot path: it reads the `aps` table, trilaterates, and returns. It never blocks on a remote API. Concretely:

1. Take current stable MACs (top-N by RSSI, passing debounce).
2. Look them up in the local `aps` cache.
3. If at least one hit, trilaterate and respond. If all missing AND we are using fallback candidates (cold-start with no stable APs yet), kick off a background resolution and return either a "no cached positions" error or, if available, the previous known fix flagged `stale: true`.

Background resolution (`resolver::resolve_background`, run from the scan loop and from the cold-start branch above) walks the provider chain in this order:

1. **Apple WPS** — free, batched, called first.
2. **WiGLE** — for BSSIDs Apple did not return; respects daily quota.

Successful lookups are written through to `aps` with a stored **source priority** (Apple > WiGLE > BeaconDB > Manual). Higher-priority writes overwrite lower-priority ones; a later WiGLE response cannot clobber an Apple fix for the same BSSID, but `last_seen` always advances. Concurrent lookups for the same BSSID are de-duplicated through an in-memory `inflight` set so the scan loop, pending drain, and locate cold-start cannot triple-fetch the same MAC.

A separate background task drains the `pending` queue periodically; entries graduate to `aps` on success, to `not_found` on authoritative miss, and are dropped after `--pending-max-attempts` failures.

## Trilateration

Weighted centroid method:

```
position = sum(weight_i * position_i) / sum(weight_i)
```

Where `weight_i` is derived from signal strength (if available):
- `weight = 10 ^ (signal_dbm / -20)` — closer APs (stronger signal) get more weight
- If no signal info (e.g. `resolve` command), equal weight

This is simple and good enough for ~20-50m urban accuracy. Not worth implementing least-squares or Kalman filtering unless proven insufficient.

## Configuration

All operational parameters are **CLI args only** — no config file for these. Secrets (API keys) come from a TOML config file.

### CLI args (with defaults)

```
whereamid \
  --bind 127.0.0.1:4747 \
  --db /var/lib/whereami/aps.sqlite \
  --interface wlan0 \
  --config ~/.config/whereami/config.toml \
  --scan-interval-fast 10 \       # seconds, during fast phase
  --scan-fast-duration 60 \       # seconds, how long fast phase lasts
  --scan-interval-slow 60 \       # seconds, steady state
  --debounce-window 10 \          # number of samples in ring buffer
  --debounce-threshold 5 \        # min appearances to be "stable"
  --top-n 10 \                    # only consider N strongest APs per scan
  --pending-interval 300 \        # seconds between pending queue drain runs
  --pending-max-attempts 20 \    # drop from pending after this many failures
  --daily-limit 100               # self-imposed WiGLE calls per day
```

### Config file (secrets only)

`~/.config/whereami/config.toml`:

```toml
[wigle]
api_user = "AID..."
api_key = "..."

[beacondb]
enabled = true
```

## Crate Structure

```
whereami/
  Cargo.toml          (workspace)
  shell.nix           (dev environment)
  nix/
    package.nix       (rustPlatform.buildRustPackage)
    module.nix        (NixOS service module)
  whereamid/          (binary — the daemon)
    src/
      main.rs         (arg parsing, background tasks, TCP accept loop)
      server.rs       (TCP connection handling, protocol dispatch, address cache)
      scanner.rs      (Wi-Fi scan via iw, top-N RSSI filtering)
      debounce.rs     (sliding window, stable AP detection)
      resolver.rs     (provider chain orchestrator, in-flight dedup)
      provider.rs     (Provider enum: Apple / WiGLE; outcome shape)
      pending.rs      (background pending queue drain task)
      trilaterate.rs  (weighted centroid)
      db.rs           (SQLite: aps + pending + not_found + metadata tables)
      config.rs       (CLI args via clap + TOML secrets)
      http.rs         (shared reqwest client builder with explicit timeouts)
      apple.rs        (Apple WPS protobuf client)
      wigle.rs        (WiGLE API client)
      beacondb.rs     (BeaconDB API client; currently unused)
      nominatim.rs    (OSM reverse geocoding, 1 req/s rate-limited)
  whereami-client/    (library crate)
    src/
      lib.rs          (connect, send command, parse response)
```

## NixOS Deployment

The project includes a `shell.nix` for development and a NixOS module for deployment.

### Development shell (`shell.nix`)

Provides: `rustc`, `cargo`, `pkg-config`, `openssl`, `sqlite`, `iw`. No flake — plain `shell.nix` to keep it simple.

### NixOS module

A NixOS module at `nix/module.nix` that provides:

```nix
services.whereami = {
  enable = true;
  bind = "127.0.0.1:4747";
  wifiInterface = "wlan0";
  wigle = {
    apiUser = "";    # or read from a secrets file
    apiKey = "";
  };
  dailyLimit = 100;
};
```

This generates:
- A systemd service (`whereamid.service`) running as a dedicated `whereami` user
- `AmbientCapabilities = CAP_NET_ADMIN` so the daemon can trigger Wi-Fi scans without running as root
- `StateDirectory = whereami` for the SQLite database (`/var/lib/whereami/aps.sqlite`)
- Hardened: `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`, etc.

### Package (`nix/package.nix`)

Standard `rustPlatform.buildRustPackage` derivation. Wraps the binary with `makeWrapper` to ensure `iw` is on `PATH`.

## Operational notes

The implementation enforces a few invariants that are not obvious from the protocol surface:

- **Explicit HTTP timeouts.** Every outbound HTTP client (Apple WPS, WiGLE, BeaconDB, Nominatim) is built through a single `client_with_timeout` helper that sets both a connect timeout and a total request timeout. `reqwest::Client::new()` with no timeout is forbidden because a hung remote would otherwise pin a tokio task indefinitely. Timeouts are currently compile-time constants (15 s for fast endpoints, 30 s for Nominatim); making them CLI-configurable is straightforward future work if needed.
- **Source-priority enforcement on upserts.** `aps.source_priority` is stored alongside the `source` string. `upsert_ap` only overwrites the data fields when the incoming source's priority is greater than or equal to the stored row's. `last_seen` always advances. A later, lower-quality WiGLE write cannot clobber an Apple fix.
- **In-flight de-duplication.** The scan loop, the pending drain, and a cold-start `locate` can all decide to resolve the same BSSID at the same time. `resolve_chain` claims each BSSID through an in-memory `inflight` set (RAII guard, panic-safe) so concurrent callers see a single remote round-trip; the losing caller defers to the cache once the winner finishes.
- **Schema invariants.** `schema_version` is constrained to a single row (`CHECK (id = 1)`). Migrations are idempotent and run inside transactions.
- **Decoupled reverse geocoding.** When `--address-approx` is set, `locate` uses an in-memory bounded cache keyed by rounded (lat, lon). Cache misses kick off a background Nominatim call so the response itself never blocks on the 1 req/s rate limiter.

## Future work

The following are explicitly **not yet built**:

- **Location history (timeseries DB).** Tracked under `task-0031`. The intent is a long-running record of resolved positions for personal analytics; nothing in the current daemon writes to such a store.
- **Continuous / streaming locate mode.** The TCP protocol is one-shot today; no subscriptions.
- **CLI-tunable HTTP timeouts.** Currently constants; would be a small extension to `Args`.
- **Bluetooth or cell-tower observations.** Wi-Fi only.
- **Active BeaconDB integration.** The client exists but is unused in the resolver chain.

## Non-goals

- No HTTP/REST — TCP + JSON lines is simpler and sufficient
- No GUI — CLI and programmatic access only
- No real-time tracking / continuous mode (yet)
- No data upload/contribution to WiGLE or BeaconDB
- No Bluetooth or cell tower support (Wi-Fi only for now)

## Open Questions

1. **WiGLE rate limits**: the exact daily free limit isn't documented clearly. We detect 429s, back off, and track remaining quota in the `metadata` table.
2. **MAC randomization**: modern devices randomize their own MAC. This doesn't affect us (we're looking up *AP* BSSIDs, not client MACs), but worth noting.
