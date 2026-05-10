# whereamid wire protocol

Canonical reference for clients in any language. Both `README.md` and `PRD.md`
link here rather than describing the protocol redundantly.

## Transport

- TCP, default `127.0.0.1:4747` (configurable via `--bind`).
- One-shot connection: connect, send one JSON object as a UTF-8 line
  terminated by `\n`, read one JSON response as a UTF-8 line, close.
- Maximum request size: **64 KiB**. Larger payloads are rejected with
  `invalid request: EOF while parsing a value`.
- Read timeout: **5 s** (server gives up if the request line doesn't arrive).
- Write timeout: **5 s** (server gives up if the client reads slowly).

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
- `"api"` — fetched from WiGLE on this call.
- `"not_found"` — no provider had a position for this BSSID.

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
    "db_write_failures": 0
}
```

- `inflight` — provider lookups currently in-flight (the dedup set).
- `db_write_failures` — cumulative best-effort DB write failures since
  daemon start. Non-zero values indicate silent corruption is being logged
  but not fatal.

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

### Bash + `nc` / `socat`

```bash
printf '{"cmd":"locate"}\n' | nc -q1 127.0.0.1 4747
echo '{"cmd":"history","range":"7d"}' | socat - TCP:127.0.0.1:4747
```

### Python (no dependencies)

```python
import json, socket
with socket.create_connection(("127.0.0.1", 4747)) as s:
    s.sendall(b'{"cmd":"locate"}\n')
    s.shutdown(socket.SHUT_WR)
    line = s.makefile("r").readline()
print(json.loads(line))
```

### Rust (typed client library)

```rust
use whereami_client::WhereAmIClient;
let resp = WhereAmIClient::default_addr().locate()?;
println!("{:.6}, {:.6}  ±{}m", resp.lat, resp.lon, resp.accuracy_m as i64);
```

## Versioning

The protocol version field (`v`) is integer-bumped on incompatible changes.
Backward-compatible additions (new fields, new commands) keep `v=1`.
