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
- Maximum request size: **64 KiB**. Larger payloads are rejected with
  `invalid request: EOF while parsing a value`.
- Read timeout: **5 s** (server gives up if the request line doesn't arrive).
- Write timeout: **5 s** (server gives up if the client reads slowly).

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
    "visible": 12, "stable": 11,
    "address": "Magstræde 5, …",     // present iff --address-approx
    "stale": false                    // true when returning the previous
                                      //   known fix because no current
                                      //   trilateration was possible
                                      // "age_s": <secs> (only when stale)
}
```

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
  from the most recent scan.
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

## Error response shape

```json
{"ok": false, "v": 1, "error": "human-readable description"}
```

Common errors:

- `unknown command: <cmd>` — the request used a `cmd` value not in the
  list above.
- `invalid request: …` — JSON parsing failed (malformed body, exceeded
  64 KiB limit, etc.).
- `resolve requires non-empty bssids array`
- `resolve accepts at most 256 BSSIDs per request (got N)`
- `history: 'range' and 'from'/'to' are mutually exclusive`
- `history: invalid range: …` (parse failure)
- `history: 'from' must be before 'to'`

## Minimal client examples

### `whereami` CLI (the official client)

```bash
whereami locate                # parsed response, friendly output
whereami --json locate         # raw JSON
whereami history 24h
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
