# whereamid wire protocol

Canonical reference for clients in any language. Both `README.md` and `PRD.md`
link here rather than describing the protocol redundantly.

## Transport

The canonical transport is a **Unix domain socket**. Access is gated by
filesystem permissions on the socket — the Unix-native access-control model (see
[ADR-003](adr/003-unix-socket-access-control.md)). This replaced the original
loopback-TCP transport ([ADR-001](adr/001-tcp-json-lines.md)) because *any* local
user could `nc 127.0.0.1:4747` and read location history, live position and
Wi-Fi scans, and drive WiGLE lookups under the owner's credentials — TCP on
localhost has no peer authentication.

- One-shot connection: connect, send one JSON object as a UTF-8 line terminated
  by `\n`, read one JSON response as a UTF-8 line, close.
- Request read cap: **64 KiB**. The line reader gives `serde_json` at most the
  first 65,536 bytes; this is not a separate physical-payload length check. If
  the JSON or terminating newline crosses the cap, the daemon parses the capped
  prefix, which normally produces `invalid request: <parser detail>`. The exact
  detail and line/column are input- and parser-version-dependent. Clients must
  keep the complete request line within the cap.
- Read timeout: **5 s**. If a complete request line has not arrived, the server
  writes exactly `{"ok":false,"v":1,"error":"read timeout"}\n` and closes the
  connection.
- Write timeout: **5 s** (server gives up if the client reads slowly).
- Active connection cap: **32** by default, configurable with
  `--max-connections` in the range **1–4096**. Unix and optional TCP connections
  share this cap; excess clients stay in the kernel listen backlog rather than
  occupying Tokio tasks.

### Socket path (daemon)

The daemon serves on a unix socket, chosen at startup in this order:

1. `--socket <path>` if given.
2. else `$XDG_RUNTIME_DIR/whereamid.sock` if `XDG_RUNTIME_DIR` is set.
3. else `/run/whereami/whereamid.sock`.

When the daemon self-binds, it tightens the socket to mode **0600** (owner-only)
and fails to start if the mode does not take (a FUSE/network filesystem that
ignores `chmod` must not silently leave the control channel exposed). A stale
socket file from a crashed daemon is detected (connect refused) and removed; a
socket a *live* daemon still owns is never stolen — startup fails instead.

**systemd socket activation.** When `whereamid` is started with `LISTEN_FDS`/
`LISTEN_PID` set, it adopts the passed fd (fd 3) as its listener and skips
unlink/chmod/cleanup entirely (systemd owns the socket file). The NixOS system
service uses this: systemd creates `/run/whereami/whereamid.sock` as
`root:whereami` mode **0660** *before* the (DynamicUser) daemon starts, so
members of the `whereami` group can connect without the daemon needing a fixed
UID. `LISTEN_FDS > 1` is a startup error.

### Access-control model

- **Self-bound (home-manager / manual):** socket is mode 0600 under
  `$XDG_RUNTIME_DIR` (itself 0700, user-owned) — single-user by construction.
- **System service:** socket is mode 0660 `root:whereami`; the operator lists
  the permitted users in `services.whereami.allowedUsers`, which populates the
  `whereami` group. Non-members get `EACCES`/connection-refused on connect.

Every user who can connect receives the same full command set, including the
destructive `wipe` command. There is no read-only or per-command role; the wipe
confirmation literal prevents accidents but is not an authorization layer.

The daemon logs the connecting peer's `SO_PEERCRED` (uid/gid/pid) at debug level
for auditability but does **not** enforce same-UID in-process — that would break
the system-service group model, where systemd hands a group-readable socket to a
DynamicUser daemon whose UID legitimately differs from the caller's. Filesystem
permissions on the socket are the sole access-control mechanism.

### Client socket resolution

The `whereami` CLI / `whereami-client` library resolve the daemon target as:

1. `$WHEREAMI_SOCKET` — a unix socket path, used verbatim (no existence check).
2. `$WHEREAMI_ADDR` — a TCP `host:port` (for old TCP daemons / custom setups).
3. the first EXISTING of `$XDG_RUNTIME_DIR/whereamid.sock`,
   `/run/whereami/whereamid.sock`.
4. otherwise an error naming the probed paths and the two overrides.

There is **no** automatic fallback to `127.0.0.1:4747`: after the daemon vacates
that port any local user could squat it and impersonate the daemon (an integrity
downgrade). Point the client at an old TCP daemon explicitly via `WHEREAMI_ADDR`.

### TCP (opt-in, unauthenticated)

TCP is available via `--bind <host:port>` but is **off by default**. It has no
peer authentication: every local user (loopback) — or, for a non-loopback
address, every reachable host — can read location history / live position /
Wi-Fi scans and drive WiGLE quota. The daemon warns loudly at startup when TCP
is enabled, and **refuses** a non-loopback bind unless `--allow-nonlocal-bind`
is also passed. Prefer the unix socket; use TCP only for deliberate remote
access you have otherwise secured.

## Request format

A request is a single JSON object with a `cmd` field selecting the command.
Per-command fields are documented below; unknown fields are silently ignored
(forward-compat) but unknown `cmd` values produce an error.

```json
{"cmd": "locate"}
{"cmd": "resolve", "bssids": ["AA:BB:CC:DD:EE:FF"]}
{"cmd": "history", "range": "7d"}
{"cmd": "wipe", "confirm": "delete-all-location-data"}
```

## Response format

Every response includes:

- `ok`: `true` for success, `false` for protocol or runtime errors.
- `v`: protocol version integer (currently `1`).
- On `ok=false`: `error` is a human-readable string.

## Commands

### `locate` — current best-known position

```json
→ {"cmd": "locate"}
← {
    "ok": true, "v": 1,
    "lat": 55.707250, "lon": 12.585598, "accuracy_m": 11.95,
    "sources": 6, "cached": 6, "fetched": 0, "pending": 0,
    "visible": 12, "stable": 11
}
```

The three conditional fields use omission, not `null` or a false sentinel:

- `address` is omitted when unavailable. `--address-approx` enables rounded-cell
  cache lookup and background geocoding, but does not guarantee this field: a
  cache miss schedules work for later and the current response still omits it.
  A stale fix includes `address` whenever that saved fix has one, regardless of
  the daemon's current `--address-approx` setting.
- `stale` is present with value `true` only when current trilateration failed and
  the daemon returned its previous fix. It is omitted from a fresh fix; the
  daemon never emits `"stale": false`.
- `age_s` is present only with a stale fix and is its non-negative age in
  seconds. It is omitted from a fresh fix.

### `scan` — currently visible Wi-Fi networks

```json
→ {"cmd": "scan"}
← {
    "ok": true, "v": 1,
    "networks": [
      {"bssid": "AA:BB:CC:DD:EE:FF", "ssid": "MyWiFi", "signal_dbm": -65, "channel": 6}
    ],
    "scan_age_ms": 3200,
    "scanned_at": "2026-05-10T09:04:38+00:00"
}
```

Presence rules:

- Each network always contains `ssid` and `channel`; either value is `null` when
  the scanner did not supply it.
- `scan_age_ms` is always present and is `null` before the first completed scan.
- `scanned_at` is omitted before the first completed scan; afterward it is an
  RFC3339 string.

### `resolve` — look up specific BSSIDs (cache-only + WiGLE fallback)

```json
→ {"cmd": "resolve", "bssids": ["AA:BB:CC:DD:EE:FF", "11:22:33:44:55:66"]}
← {
    "ok": true, "v": 1,
    "results": [
      {"bssid": "AA:BB:CC:DD:EE:FF", "lat": 48.857, "lon": 2.351,
       "ssid": "MyWiFi", "source": "cache"},
      {"bssid": "11:22:33:44:55:66", "lat": null, "lon": null,
       "ssid": null, "source": "not_found"}
    ]
}
```

`source` in this response is the **provenance** for the result:
- `"cache"` — answered from the local `aps` table.
- `"api"` — fetched from a provider on this call.
- `"not_found"` — **authoritative absence**: a provider returned NotFound for
  this BSSID on this call, or the daemon's not_found table recorded absence
  within its TTL. Safe to cache as "not locatable".
- `"unresolved"` — the daemon could **not** consult a provider for this BSSID
  this call, so absence is **not** established. Causes: the lookup was deferred
  to an in-flight background resolution of the same BSSID; the only provider
  was skipped because WiGLE credentials are unconfigured or the daily quota is
  exhausted; or a transient network / rate-limit failure. Retry later — do
  **not** cache this as absent.

(This is distinct from the `aps.source` column in the SQLite cache, which
records *which provider* originally supplied the position: `apple`, `wigle`,
`beacondb`, `manual`, or `unknown`.)

Maximum **256 BSSIDs** per `resolve` request.

### `stats` — cache + observability counters

```json
→ {"cmd": "stats"}
← {
    "ok": true, "v": 1,
    "cached_aps": 42,
    "pending_aps": 0,
    "not_found_aps": 3,
    "db_size_bytes": 49152,
    "api_calls_today": 5,
    "inflight": 0,
    "db_write_failures": 0,
    "orphan_tasks_aborted": 0
}
```

- `inflight` — provider lookups currently in-flight (the dedup set).
- `db_write_failures` — cumulative best-effort DB write failures since
  daemon start. Non-zero values indicate silent corruption is being logged
  but not fatal.
- `orphan_tasks_aborted` — number of background tasks (proactive resolve /
  address backfill spawns and the daemon's scan/drain/prune/server loops)
  force-aborted at the end of the **last clean shutdown** because they did
  not finish within the drain budget. Non-zero means that shutdown killed
  in-flight work. The value is persisted at shutdown and reloaded at
  startup; a crash/SIGKILL does not update it.

### `debug` — daemon debug snapshot

```json
→ {"cmd": "debug"}
← {
    "ok": true, "v": 1,
    "daemon_rev": "abc1234",
    "scan_age_ms": 3200,
    "samples_in_buffer": 5,
    "visible": 12,
    "stable": 11,
    "bssids": [
      {"bssid": "AA:BB:CC:DD:EE:FF", "signal_dbm": -65, "seen": 5,
       "needed": 5, "is_stable": true, "db_status": "cached"},
      {"bssid": "11:22:33:44:55:66", "signal_dbm": null, "seen": 5,
       "needed": 5, "is_stable": true, "db_status": "new"}
    ]
}
```

- `signal_dbm` is `null` when the BSSID is debounce-stable but absent
  from the most recent scan; the key itself is not omitted.
- `scan_age_ms` is always present and is `null` before the first completed scan.
- `db_status` ∈ `{cached, pending, not_found, new}`.

### `version` — daemon version + git revision

```json
→ {"cmd": "version"}
← {"ok": true, "v": 1, "version": "0.4.0", "git_rev": "abc1234"}
```

### `history` — stay-point segments from the location-history timeseries

```json
→ {"cmd": "history", "range": "7d"}
← {
    "ok": true, "v": 1,
    "from": "2026-05-03T09:00:00+00:00",
    "to":   "2026-05-10T09:00:00+00:00",
    "segments": [
      {
        "start_rfc3339": "2026-05-09T08:00:00+00:00",
        "end_rfc3339":   "2026-05-09T17:30:00+00:00",
        "duration_secs": 34200,
        "centroid_lat":  55.707250,
        "centroid_lon":  12.585598,
        "mean_accuracy_m": 11.95,
        "n_fixes": 142
      }
    ]
}
```

Range parameters (mutually exclusive):
- `"range": "<N><unit>"` — relative duration ending at now. Units: `s`,
  `m`, `h`, `d`, `w`. Examples: `"7d"`, `"24h"`, `"30m"`, `"1w"`.
- `"from"` + `"to"` — absolute RFC3339 timestamps.

If neither is provided the default is the last 7 days.

Segments group consecutive fixes within `--history-segment-distance-m`
(default 100 m) into one stay-point and drop runs shorter than
`--history-segment-min-duration-secs` (default 300 s).

History is single-flight. While one history request is querying, segmenting, or
writing its response, another receives `history: busy; retry later` promptly.
Segmentation and JSON serialization run off the async executor and are bounded
to O(N) work, but one admitted request still allocates its O(N) database result
and response. The concurrency cap is not an absolute history-row or memory cap.

### `wipe` — delete logical location and observed-network state

`wipe` is destructive and is accepted only over the filesystem-permission-gated
Unix socket. The daemon refuses it over the unauthenticated TCP transport,
including loopback. The confirmation string is literal and mandatory, but it
is an accident guard rather than separate authorization:

```json
→ {"cmd":"wipe","confirm":"delete-all-location-data"}
← {
    "ok": true, "v": 1,
    "aps_deleted": 8,
    "pending_deleted": 2,
    "not_found_deleted": 3,
    "fixes_deleted": 144,
    "last_fix_deleted": 1
}
```

One SQLite `IMMEDIATE` transaction deletes `aps`, `pending`, `not_found`,
`fixes`, `last_fix`, and the `fixes` row in `sqlite_sequence`. Each response
count is the number of rows deleted from its named table; a repeated wipe is
successful with zero counts. `schema_version` and operational `metadata`
(including quota and shutdown observability) are retained. Configuration and
credentials are outside these tables and are untouched. A database error rolls
back the transaction and leaves managed in-memory state unchanged.

After commit, the daemon clears managed in-memory scan/debounce, last-fix,
address-cache, geocode-queue, and in-flight lookup state while exclusive, then
advances the publication epoch last. The boundary remains exclusive until the
response-write attempt terminates by successful write, write failure, timeout,
or disconnect. No post-wipe publication can precede that termination, and an
older response that lost the race becomes
`request invalidated by data wipe; retry`.

Wipe is single-flight across that entire boundary. An overlapping request gets
`wipe: busy; retry later` rather than queuing another O(N) deletion. A client
disconnect does not cancel an accepted wipe, but a missing complete response
leaves its result unknown; verify the state or retry idempotently. Collection
remains enabled and may repopulate the database after the response attempt
terminates. Wipe is logical live-state deletion, not forensic erasure of SQLite
free pages/WAL remnants, allocator/process-memory remnants outside the managed
structures named above, snapshots, backups, storage media, logs, or provider
traffic already sent.

An internal invariant panic after SQLite commit is not recoverable in-process:
the daemon retains the exclusive publication boundary through fatal handling,
logs the failure, and exits nonzero before it can resume serving a
persisted/in-memory split. Under systemd it can then restart against the
already-wiped database. To the client this is an interrupted, unknown result,
so the same verify-or-retry rule applies.

## Error response shape

```json
{"ok": false, "v": 1, "error": "human-readable description"}
```

Common errors:

- Unknown and missing commands are `invalid request` parser errors; there is no
  separate `unknown command: …` error. For the exact compact request
  `{"cmd":"bogus"}`, the current daemon emits this exact response line (plus
  its framing newline):

  ```json
  {"ok":false,"v":1,"error":"invalid request: unknown variant `bogus`, expected one of `locate`, `resolve`, `scan`, `stats`, `debug`, `version`, `history`, `wipe` at line 1 column 14"}
  ```

  For `{"range":"7d"}` (no `cmd`), it emits:

  ```json
  {"ok":false,"v":1,"error":"invalid request: missing field `cmd` at line 1 column 14"}
  ```

  These details come from the current `serde_json` parser. The accepted-variant
  list changes when commands are added, and line/column values depend on the
  exact input.
- `invalid request: <parser detail>` — other JSON/deserialization failures,
  including malformed or truncated request bodies. The prefix is stable; parser
  detail and its line/column suffix depend on the input and parser version.
- `read timeout` — a complete request line did not arrive within 5 seconds. The
  exact response is `{"ok":false,"v":1,"error":"read timeout"}`.
- `resolve requires non-empty bssids array`
- `resolve accepts at most 256 BSSIDs per request (got N)`
- `history: 'range' and 'from'/'to' are mutually exclusive`
- `history: invalid range: <parser detail>`
- `history: invalid 'from' timestamp: <parser detail>`
- `history: invalid 'to' timestamp: <parser detail>`
- `history: 'from' must be before 'to'`
- `history: provide either 'range' or both 'from' and 'to'`
- `history: busy; retry later` — another history query/response is in flight
- `history: cancelled` — shutdown cancelled admitted history work
- `history: processing failed` — the blocking history worker failed
- `history: db error: <detail>` — the history database query failed
- `wipe: confirmation must equal 'delete-all-location-data'`
- `wipe: unavailable over unauthenticated TCP; use the Unix control socket`
- `wipe: busy; retry later` — another wipe remains admitted through its
  response-write attempt
- `wipe: result unavailable; verify and retry` — completion could not be
  reported; its result is unknown and an idempotent retry is safe
- `wipe: db error: <detail>` — deletion failed; no managed memory was cleared
- `request invalidated by data wipe; retry` — a privacy-bearing request began
  before a concurrent successful wipe and lost the response-publication race

The six constant history strings (mutual exclusion, ordering, missing pair,
busy, cancelled, and processing failed) are exact. The four dynamic forms are
the three `invalid` prefixes and `history: db error:`; their suffixes come from
the range parser, Chrono's RFC3339 parser, or the database stack and can vary
with the failing value and dependency version.

Concrete current parser examples:

- `{"cmd":"history","range":"bogus"}` produces
  `history: invalid range: invalid range 'bogus': expected '<number><unit>' (e.g. '7d')`.
- `{"cmd":"history","from":"not-a-time","to":"2026-07-05T01:00:00Z"}`
  produces `history: invalid 'from' timestamp: premature end of input`.
- `{"cmd":"history","from":"2026-07-05T00:00:00Z","to":"not-a-time"}`
  produces `history: invalid 'to' timestamp: premature end of input`.

Those complete strings are exact for the current inputs and dependency
versions, not stable parser codes; in particular, the parser-generated suffixes
may change after a dependency update.

All `error` values are human-readable diagnostics, not machine-stable error
codes. The exact strings and prefixes above describe and regression-test the
current daemon; clients should branch on `ok`, not parse error prose.

## Official CLI `--json` behavior

The client transport removes the framing newline and trims surrounding
whitespace from the daemon response. For any syntactically valid JSON body that
remains, `whereami --json …` writes that body byte-for-byte and adds one output
newline. It does not deserialize and re-serialize the command-specific response,
so unknown fields and the distinction between omitted fields and `null` survive.
The exit status is 0 if and only if the top-level `ok` field is the JSON boolean
`true`; every other value, a missing `ok`, or `ok: false` exits 1.

This deliberately creates an asymmetry with human output. JSON mode checks only
that the body is valid JSON and that `ok` is boolean `true`; it does not validate
`v` or the command-specific schema. An otherwise schema-invalid response with
`ok: true` can therefore pass through and exit 0. Human mode uses the typed
response for that command and can reject the same line during deserialization.

If no daemon line is available because transport failed, or if the daemon line
is not valid JSON, the CLI emits its own valid JSON error object and exits 1:

```json
{"error":"description","ok":false}
```

This CLI-only envelope intentionally has no protocol `v` field and is not a
daemon response. JSON object key order is not a contract. Clap parse errors
(such as an unknown flag) occur before target resolution: even under `--json`,
they write to stderr, leave stdout empty, and exit 1. The history `--from`/`--to`
pair check occurs after target resolution. With a resolved target it likewise
writes to stderr with empty stdout and exits 1, but if no target can be resolved,
the earlier target-resolution failure instead produces the synthesized JSON
envelope on stdout and exits 1. This precedence is tracked for a future decision
in TASK-0228.

The 0/1 rules above apply when I/O completes. On Unix, an early-closed stdout
pipe (or a daemon connection that closes during a request write) can instead
terminate the CLI via `SIGPIPE`; shells conventionally report status **141**
(`128 + SIGPIPE(13)`).

## Minimal client examples

### `whereami` CLI (the official client)

```bash
whereami locate                # parsed response, friendly output
whereami --json locate         # raw JSON
whereami history 24h
whereami wipe --yes            # Unix socket only; destructive confirmation
```

### Bash + `nc` / `socat` (unix socket)

```bash
SOCK="${XDG_RUNTIME_DIR:-/run/whereami}/whereamid.sock"
printf '{"cmd":"locate"}\n' | nc -q1 -U "$SOCK"
echo '{"cmd":"history","range":"7d"}' | socat - UNIX-CONNECT:"$SOCK"
```

### Python (no dependencies)

```python
import json, os, socket
sock_path = os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/run/whereami"),
                         "whereamid.sock")
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
    s.connect(sock_path)
    s.sendall(b'{"cmd":"locate"}\n')
    s.shutdown(socket.SHUT_WR)
    line = s.makefile("r").readline()
print(json.loads(line))
```

### Rust (typed client library)

```rust
use whereami_client::WhereAmIClient;
// Resolves the socket from the environment (WHEREAMI_SOCKET / WHEREAMI_ADDR /
// the default paths). Use WhereAmIClient::unix(path) or ::tcp(addr) to pin one.
let resp = WhereAmIClient::from_env()?.locate()?;
println!("{:.6}, {:.6}  ±{}m", resp.lat, resp.lon, resp.accuracy_m as i64);
```

(For an old TCP-only daemon, export `WHEREAMI_ADDR=127.0.0.1:4747` or use
`WhereAmIClient::tcp("127.0.0.1:4747")`.)

## Versioning

The protocol version field (`v`) is integer-bumped on incompatible changes.
Backward-compatible additions (new fields, new commands, and new enum values
in existing string fields such as `source`) keep `v=1`. Clients should treat
an unrecognised `source` value like `unresolved` (absence not established, so
retry) rather than erroring. The `source: "unresolved"` value was added this
way (task-0194).
