> **ARCHIVED — pre-retirement PRD (v0.x detailed).** Retired by task-0113 because this document repeatedly drifted from reality; kept here for history, not maintained.
> The authoritative sources of truth are now `README.md`, `docs/protocol.md`, and `CHANGELOG.md`. See the lean `PRD.md` at the repo root.

# whereami — Local Wi-Fi Geolocation Service

## Problem

Browser-based geolocation (Google, Mozilla) requires sending your Wi-Fi environment to third-party servers on every query. Offline geolocation on Linux doesn't exist in usable form. GeoClue's Mozilla backend is dead.

We want: scan local Wi-Fi, resolve MAC addresses to coordinates, cache results locally, and answer "where am I?" without phoning home once the cache is warm.

## Architecture

Two components:

```
                 unix socket (fs-perm gated)
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

HTTP timeouts are CLI-configurable via `--http-timeout-secs` (default 15s,
applies to Apple WPS and WiGLE) and `--nominatim-timeout-secs` (default
30s). The reverse-geocoded address cache TTL is configurable via
`--address-cache-ttl-days` (default 7; 0 disables caching).

### whereamid (daemon)

Long-running process. Listens on a Unix domain control socket gated by filesystem permissions (default `$XDG_RUNTIME_DIR/whereamid.sock`; TCP via `--bind` is opt-in and unauthenticated — see [ADR-003](docs/adr/003-unix-socket-access-control.md)). Newline-delimited JSON protocol (one JSON object per line, one JSON response per line). No HTTP overhead — just raw sockets + JSON lines.

Responsibilities:
- Accept queries over the filesystem-permission-gated Unix socket and optional TCP
- Manage the local SQLite cache (read/write)
- Perform Wi-Fi scans (via `nmcli` preferred, `iw dev wlan0 scan` fallback)
- Call remote APIs on cache miss (Apple WPS first — free, no auth — then WiGLE)
- Trilaterate position from known AP locations using a spherical-mean centroid
- Atomically reserve outbound WiGLE API calls against a daily quota
- Persist last-known fix and a fix timeseries (location-history feature)
- Drain pending lookups in the background when connectivity returns
- Prune fix history and AP observations under independently configurable
  retention windows (30 days by default; 0 keeps the table indefinitely)
- Provide a confirmed, Unix-socket-only wipe of logical location/AP state
- Drop `--address-approx` reverse-geocode calls onto a TTL'd cache so the
  locate hot path never blocks on Nominatim's 1 req/s rate limit

### whereami-client (Rust library crate)

Typed client for the Unix control socket, with explicit opt-in TCP support.
Connect, send JSON, and read one JSON response. Other languages can open the
Unix socket and use the same JSON-lines protocol without a client library.

## Control Protocol

Request and response are each a single line of JSON terminated by `\n`.

**Connection lifecycle**: one-shot. Client connects, sends one JSON line, reads one JSON response line, connection closes. No persistent connections, no request multiplexing. At most `--max-connections` connections (range 1–4096, default 32) are active across the Unix and optional TCP listeners; excess connections remain in the kernel listen backlog. Localhost TCP setup is sub-millisecond so the overhead is negligible.

All responses include a `"v": 1` field for protocol versioning.

### Commands

**`locate`** — "Where am I right now?"

Cache-only hot path: takes the current stable APs (debounce filter), trilaterates from whatever positions are already in the local `aps` cache, and responds immediately. Remote provider calls happen in background tasks (scan loop, pending drain, cold-start fallback) — `locate` itself never awaits an external API. If no stable APs have cached positions, the response is either an error or, if available, the previous known fix flagged with `stale: true` and `age_s`.

If `--address-approx` is enabled, an approximate street address is attached when the rounded position is in the local address cache. A miss publishes bounded, latest-only background work; a later locate at the same rounded position may include the address only after a successful, still-current result populated the cache.

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

**`wipe`** — Delete logical location and observed-network state. This command
is accepted only over the filesystem-permission-gated Unix socket and requires
the exact confirmation literal. Every user authorized to connect to that socket
can wipe; there is no read-only or per-command role. The literal prevents
accidental invocation but is not an additional authorization layer.

```json
{"cmd":"wipe","confirm":"delete-all-location-data"}
```

Response:
```json
{"ok":true,"v":1,"aps_deleted":1,"pending_deleted":1,"not_found_deleted":1,"fixes_deleted":12,"last_fix_deleted":1}
```

The transaction also resets the `fixes` AUTOINCREMENT sequence, while retaining
schema and operational metadata. After commit, all managed in-memory location
state is cleared while exclusive and the publication epoch advances last. The
exclusive boundary ends when the response-write attempt terminates by success,
failure, timeout, or disconnect; collection remains enabled and can then
repopulate. If no complete response arrives, completion is unknown: verify or
retry the idempotent wipe. The operation is logical deletion, not forensic
erasure of WAL/free pages, allocator/process-memory remnants outside managed
caches, snapshots, backups, logs, or storage media. A panic in the post-commit
memory/epoch continuation retains the publication boundary through fail-stop
and exits nonzero; the daemon must never recover only its database actor and
serve split persisted/in-memory state.

### Error shape

```json
{"ok": false, "error": "wifi scan failed: permission denied"}
```

## Data Model (SQLite)

Schema version is **7**. All timestamps are **UTC ISO 8601 / RFC3339**.
SQLite in **WAL mode** (set on open via `PRAGMA journal_mode=WAL`). Migrations
run inside transactions and roll back atomically on failure.

```sql
-- Schema version. Single-row CHECK guards against accidental multi-row state.
CREATE TABLE schema_version (
    id      INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL
);

-- Resolved APs with known positions
CREATE TABLE aps (
    bssid           TEXT PRIMARY KEY,  -- normalized uppercase, colon-separated
    ssid            TEXT,
    lat             REAL NOT NULL,
    lon             REAL NOT NULL,
    encryption      TEXT,              -- informational, from WiGLE
    channel         INTEGER,
    frequency       INTEGER,           -- MHz
    city            TEXT,
    country         TEXT,
    source          TEXT NOT NULL,     -- "apple" | "wigle" | "beacondb" | "manual" | "unknown"
    source_priority INTEGER NOT NULL DEFAULT 0,
                                       -- 40 apple, 30 wigle, 20 beacondb (legacy), 10 manual, 0 unknown
    first_seen      TEXT NOT NULL,
    last_seen       TEXT NOT NULL,
    fetched_at      TEXT NOT NULL,
    is_mobile       INTEGER NOT NULL DEFAULT 0
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
-- Read+increment runs in a single SQL transaction (try_reserve_api_call).

-- Last-known fix, persisted across daemon restarts so `locate` can answer
-- immediately at startup before the next scan-and-resolve cycle.
CREATE TABLE last_fix (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    lat         REAL NOT NULL,
    lon         REAL NOT NULL,
    accuracy_m  REAL NOT NULL,
    address     TEXT,
    at_rfc3339  TEXT NOT NULL,
    sources     INTEGER NOT NULL
);

-- Location-history timeseries: one row per successful locate response.
CREATE TABLE fixes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    at_rfc3339  TEXT NOT NULL,
    lat         REAL NOT NULL,
    lon         REAL NOT NULL,
    accuracy_m  REAL NOT NULL,
    n_sources   INTEGER NOT NULL,
    implied_speed_kmh REAL,
    last_fix_age_secs INTEGER,
    inlier_ratio      REAL,
    confidence        TEXT,     -- "high" | "medium" | "low" | "suspect"
    suspect_reason    TEXT
);
CREATE INDEX idx_fixes_at ON fixes(at_rfc3339);
```

The five nullable quality columns are one bundle. All five `NULL` identifies a
pre-v7 legacy row with unknown quality. Every new daemon-written fix has a
non-NULL `confidence`; speed and age are both absent only when no previous fix
exists, and `suspect_reason` is present exactly for `suspect` confidence.

The **pending table** acts as a work queue. The daemon drains it in the background whenever WiGLE becomes reachable:
- On successful lookup: row moves from `pending` to `aps`, deleted from `pending`
- On WiGLE 404 (MAC genuinely unknown): row moved to `not_found`, deleted from `pending`
- On network error / 429: `attempts` incremented, row stays in `pending`
- On `attempts >= max_attempts` (default 20, configurable via `--pending-max-attempts`): row deleted from `pending`, treated as permanently unresolvable

The daemon prunes `fixes` older than `--history-retention-days` and `aps` rows
whose `last_seen` is older than `--aps-retention-days`. Both default to 30 days;
0 keeps the corresponding table indefinitely. One sweep runs immediately at
startup and then daily in alternating 1,000-row transactions. Entries in
`not_found` can be re-checked after a configurable TTL (default 30 days).

## Wi-Fi Scanning

Two backends, in preference order:

1. **Forced nmcli with NetworkManager generation proof** (preferred) — resolve
   the NetworkManager unique D-Bus owner and device path with `busctl`, baseline
   `Device.Wireless.LastScan`, then run `nmcli -t -f
   BSSID,SSID,SIGNAL,CHAN,FREQ device wifi list ifname <iface> --rescan yes`.
   Only a strictly advanced, nonfuture generation at/after the attempt followed
   by a `--rescan no` relist and same-owner/path final fence is confirmed. All
   device/property reads target the captured unique owner, and a trailing owner
   check closes the final property-read race. This runs without `CAP_NET_ADMIN`
   when NetworkManager manages the interface.
2. **Blocking `iw dev <iface> scan`** — fallback when nmcli has no usable
   result. Requires `CAP_NET_ADMIN`. Busy responses are retried; only the
   successful attempt owns evidence. A separate `scan dump` after generic or
   exhausted-busy failure is cached data and explicitly unconfirmed.

The daemon preserves non-empty cached output for interactive continuity, but
only a proved nmcli post-generation relist or successful blocking iw command is
eligible for positive autonomous freshness. Each backend returns a `ScanBatch`
containing networks plus optional `CLOCK_BOOTTIME` evidence; confirmed empty
batches are retained rather than falling through to another backend.

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

### BeaconDB (read-only legacy)

`Source::BeaconDb` exists in the enum at priority 20 so historical DB rows
from earlier prototypes still load at the correct priority. No production
path writes new BeaconDB rows today; the BeaconDbClient module was deleted
in task-0034. The API returns aggregate position (not per-AP), which makes
it incompatible with the per-BSSID Provider chain.

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

**Spherical-mean** weighted centroid:

```
v_i      = unit_vec(lat_i, lon_i)         # 3D unit vector on the sphere
weight_i = 10 ^ (signal_dbm_i / 20)        # stronger signal = larger weight
v        = normalize( sum(weight_i * v_i) )
(lat, lon) = lat_lon_of(v)
```

The 3D-vector approach is necessary because lat/lon arithmetic mean is wrong
on a sphere: two APs at lon=±179° would centroid to lon=0° (Africa) instead
of the antimeridian. The current implementation handles antimeridian and
near-polar inputs correctly. Antipodal inputs (vector sum near zero) fall
back to the strongest-signal AP and inflate `accuracy_m`.

Outlier rejection uses the same spherical-mean center plus median-distance
filtering. Accurate to ~10–30m urban with 5+ resolved APs.

## Configuration

All operational parameters are **CLI args only** — no config file for these. Secrets (API keys) come from a TOML config file.

### CLI args (with defaults)

```
whereamid \
  --socket $XDG_RUNTIME_DIR/whereamid.sock \  # default; TCP via --bind is opt-in
  --db /var/lib/whereami/aps.sqlite \
  --interface wlan0 \
  --config ~/.config/whereami/config.toml \
  --max-connections 32 \                  # shared Unix/TCP connection cap
  --scan-interval-fast 10 \              # seconds, fast phase
  --scan-fast-duration 60 \              # seconds, fast-phase length
  --scan-interval-slow 60 \              # seconds, steady state
  --debounce-window 10 \                 # ring buffer size
  --debounce-threshold 5 \               # min appearances to be stable
  --top-n 10 \                           # strongest APs for trilateration
  --pending-interval 300 \               # pending drain interval (sec)
  --pending-max-attempts 20 \            # drop pending after N failures
  --daily-limit 100 \                    # WiGLE calls per day
  --not-found-ttl-days 30 \              # re-check not_found after N days
  --address-approx \                     # include street address (Nominatim)
  --address-cache-ttl-days 7 \           # reverse-geocode cache TTL
  --http-timeout-secs 15 \               # Apple/WiGLE total timeout
  --nominatim-timeout-secs 30 \          # Nominatim total timeout
  --history-retention-days 30 \          # prune fixes older than N days
  --aps-retention-days 30 \              # prune APs not seen for N days
  --history-segment-distance-m 100 \     # stay-point clustering threshold
  --history-segment-min-duration-secs 300  # min stay-point duration
```

### Config file (secrets only)

`~/.config/whereami/config.toml`:

```toml
[wigle]
api_user = "AID..."   # optional; daemon runs without WiGLE creds
api_key  = "..."
```

## Crate Structure

```
whereami/
  Cargo.toml          (workspace)
  shell.nix           (dev environment)
  nix/
    package.nix       (Crane package plus shared build/check artifacts)
    module.nix        (NixOS service module)
  whereamid/          (binary — the daemon)
    src/
      main.rs         (arg parsing, lifecycle/background tasks, canonical Unix listener setup)
      server.rs       (shared Unix/optional-TCP accept loop, protocol dispatch, address cache)
      scanner.rs      (nmcli+busctl generation proof, blocking iw fallback, scan parsing)
      boot_clock.rs   (Linux CLOCK_BOOTTIME timestamps for suspend-inclusive freshness)
      debounce.rs     (sliding window, stable AP detection)
      resolver.rs     (provider chain orchestrator, in-flight dedup)
      provider.rs     (Provider enum: Apple / WiGLE; outcome shape)
      pending.rs      (background pending queue drain task)
      trilaterate.rs  (weighted centroid)
      db.rs           (SQLite: aps + pending + not_found + metadata tables)
      config.rs       (CLI args via clap + TOML secrets)
      http.rs         (shared reqwest client builder with explicit timeouts)
      apple.rs        (Apple WPS protobuf client + encoder/decoder)
      wigle.rs        (WiGLE API client)
      nominatim.rs    (OSM reverse geocoding, 1 req/s rate-limited)
      history.rs      (segmentation + range parsing for location history)
      geo.rs          (canonical haversine_m)
      lib.rs          (re-exports for fuzzers and integration tests)
      db/source.rs    (Source enum + priority ladder)
      server/address_cache.rs (TTL'd reverse-geocode cache)
  whereami-client/    (library + CLI)
    src/
      lib.rs          (typed request/response, WhereAmIClient)
      main.rs         (the `whereami` CLI binary)
```

## NixOS Deployment

The project includes a flake development shell, a `shell.nix` compatibility
entry point, and a NixOS module for deployment.

### Development shell (`shell.nix`)

`shell.nix` selects the local flake's `devShells.<system>.default` rather than
duplicating the environment. Both `nix-shell` and `nix develop` therefore
provide the same nightly Rust toolchain, `cargo-fuzz`, `just`, `pkg-config`,
`openssl`, `sqlite`, `iw`, NetworkManager (`nmcli`), and systemd (`busctl`)
tools.

This development workflow requires a Git checkout and flakes-enabled Nix.
The `shell.nix` `git+file`/`getFlake` compatibility path intentionally does
not support source tarballs, Nix store paths, or older flakes-disabled Nix.

### NixOS module

A NixOS module at `nix/module.nix` that provides:

```nix
services.whereami = {
  enable = true;
  # Full control-socket access, including destructive wipe; no read-only role.
  allowedUsers = [ "alice" ];
  # bind = "127.0.0.1:4747";    # opt-in, unauthenticated TCP (off by default)
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

A Crane derivation builds dependencies once with `craneLib.buildDepsOnly`, then
reuses those artifacts for `craneLib.buildPackage` and the package checks. The
installed daemon is wrapped with `makeWrapper`; its runtime `PATH` includes the
`iw`, NetworkManager, and systemd packages, which provide `iw`, `nmcli`, and
`busctl` respectively.

## Operational notes

The implementation enforces a few invariants that are not obvious from the protocol surface:

- **Explicit HTTP timeouts.** Every outbound HTTP client (Apple WPS, WiGLE, Nominatim) is built through a single `client_with_timeout` helper that sets both a connect timeout and a total request timeout. `reqwest::Client::new()` with no timeout is forbidden because a hung remote would otherwise pin a tokio task indefinitely. Total timeouts are CLI-configurable: `--http-timeout-secs` (Apple/WiGLE, default 15) and `--nominatim-timeout-secs` (default 30). Connect timeout is fixed at 5s.
- **Source-priority enforcement on upserts.** `aps.source_priority` is stored alongside the `source` string. `upsert_ap` only overwrites the data fields when the incoming source's priority is greater than or equal to the stored row's. `last_seen` always advances. A later, lower-quality WiGLE write cannot clobber an Apple fix.
- **In-flight de-duplication.** The scan loop, the pending drain, and a cold-start `locate` can all decide to resolve the same BSSID at the same time. `resolve_chain` claims each BSSID through an in-memory `inflight` set (RAII guard, panic-safe) so concurrent callers see a single remote round-trip; the losing caller defers to the cache once the winner finishes.
- **Schema invariants.** `schema_version` is constrained to a single row (`CHECK (id = 1)`). Migrations are idempotent and run inside transactions.
- **Decoupled reverse geocoding.** When `--address-approx` is set, `locate` uses an in-memory bounded cache keyed by rounded (lat, lon). Cache misses publish to a bounded, latest-only background scheduler; deduplication, supersession, shutdown, or queue age can prevent outbound dispatch, and `locate` never awaits the 1 req/s rate gate or Nominatim HTTP.

## Future work

The following are explicitly **not yet built**:

- **Continuous / streaming locate mode.** The TCP protocol is one-shot today;
  no subscriptions or long-poll.
- **Bluetooth or cell-tower observations.** Wi-Fi only.
- **Active BeaconDB integration.** The client was deleted in task-0034; the
  Source enum still recognises historical rows. A renewed integration would
  need the API to expose per-AP positions instead of aggregate fixes.
- **Mozilla Location Services / Combain / Skyhook providers.** The Provider
  trait makes adding a new backend a small change.

## Non-goals

- No HTTP/REST — TCP + JSON lines is simpler and sufficient
- No GUI — CLI and programmatic access only
- No real-time tracking / continuous mode (yet)
- No data upload/contribution to WiGLE or any other crowdsourced backend
- No Bluetooth or cell tower support (Wi-Fi only for now)

## Open Questions

1. **WiGLE rate limits**: the exact daily free limit isn't documented clearly. We detect 429s, back off, and track remaining quota in the `metadata` table.
2. **MAC randomization**: modern devices randomize their own MAC. This doesn't affect us (we're looking up *AP* BSSIDs, not client MACs), but worth noting.
