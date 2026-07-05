# whereami-client

Typed Rust client for the [whereamid](https://github.com/anthropics/whereami)
geolocation daemon. Speaks the JSON-lines protocol documented in
[docs/protocol.md](../docs/protocol.md) — a Unix domain socket by default, with
opt-in TCP.

The crate ships two artefacts:

- A library with typed request/response structs (`LocateResponse`,
  `HistoryResponse`, etc.) and a `WhereAmIClient` that handles connection
  setup and parsing.
- A `whereami` binary that wraps the library with a friendly CLI.

## Library usage

```rust
use std::time::Duration;
use whereami_client::{WhereAmIClient, WipeConfirmation};

// Resolve the daemon from the environment: WHEREAMI_SOCKET / WHEREAMI_ADDR /
// the default unix-socket paths ($XDG_RUNTIME_DIR/whereamid.sock, then
// /run/whereami/whereamid.sock).
let client = WhereAmIClient::from_env()?;

// Where am I?
let resp = client.locate()?;
println!(
    "{:.6}, {:.6}  ±{}m ({})",
    resp.lat, resp.lon, resp.accuracy_m as i64, resp.confidence
);

// History over the last 24 hours.
let h = client.history(Some("24h".into()), None, None)?;
for seg in &h.segments {
    println!("{} -> {}  {} fixes", seg.start_rfc3339, seg.end_rfc3339, seg.n_fixes);
}

// Opt in to unsegmented fixes, capped to the newest 1000 observations.
let raw = client.history_fixes(Some("24h".into()), None, None, Some(1000))?;
for fix in &raw.fixes {
    println!("{}  {:.6}, {:.6}", fix.at_rfc3339, fix.lat, fix.lon);
}

// Pin a specific transport instead of resolving from the environment:
let via_socket = WhereAmIClient::unix("/run/whereami/whereamid.sock");
let via_tcp = WhereAmIClient::tcp("10.0.0.5:4747"); // opt-in, unauthenticated
let stats = via_socket.stats()?;

// Raise the per-read/write timeout for a long request, or explicitly disable
// it when an outer supervisor provides the deadline.
let long_request = WhereAmIClient::from_env()?
    .with_io_timeout(Duration::from_secs(120))?;
let externally_supervised = WhereAmIClient::from_env()?.without_io_timeout();

// Destructive operations require an explicit confirmation marker at the call
// site. This prevents accidents but is not authorization: every user allowed
// to connect to the Unix socket can wipe. The daemon never accepts wipe on TCP.
let deleted = via_socket.wipe(WipeConfirmation::Confirmed)?;
println!("deleted {} AP rows", deleted.aps_deleted);
```

New clients default to a 40-second timeout on each blocking socket read and
write. That covers the nominal 35-second envelope for one provider chain at the
daemon defaults (up to 15 seconds for Apple, 15 seconds for WiGLE, and 5 seconds
to write the daemon response) plus 5 seconds of scheduling and database margin.
It is not a total request deadline. Multi-BSSID requests, large history queries,
or custom daemon provider timeouts can legitimately take longer; raise the
timeout or disable it only when an outer supervisor supplies the deadline.

`StatsResponse::orphan_tasks_aborted` is the number of tasks force-aborted at
the end of the last clean daemon shutdown. It is one persisted shutdown
snapshot, neither a current count nor a cumulative total. The daemon loads it on
startup; a crash or `SIGKILL` leaves the previous clean-shutdown value unchanged.
Older daemons omit the field, which the typed client reads as `0`.

`StatsResponse::autolocate_fixes_written` counts autonomous history rows
successfully committed since daemon start; manual `locate` writes are excluded.
It resets on restart and defaults to `0` when an older daemon omits the field.

`LocateResponse` exposes the daemon's additive uncertainty fields:
`implied_speed_kmh`, `last_fix_age_secs`, `inlier_ratio`, `confidence`, and
`suspect_reason`. Missing fields from an older daemon deserialize as `None` and
`Confidence::High`, preserving the behavior clients saw before explicit
plausibility reporting. The human CLI appends confidence to the coordinate
line and only emits confidence colors when stdout is a terminal; `--json`,
pipes, redirects, and the optional address line are never colored.

All typed response structs expose an `ok: bool` flag and an
`error: Option<String>` field so callers have one uniform model. Successful
daemon wire objects omit `error`; daemon error objects include it. The crate
exposes a `DaemonResponse` trait so generic dispatchers can inspect both
uniformly:

```rust
fn handle<T: whereami_client::DaemonResponse>(r: &T) {
    if !r.is_ok() { panic!("{}", r.error().unwrap_or("unknown error")); }
}
```

## CLI usage

```bash
whereami locate                          # default subcommand
whereami scan                            # currently visible Wi-Fi
whereami stats                           # cache + observability counters
whereami debug                           # daemon debug snapshot
whereami history 7d                      # stay-point segments
whereami history 7d --raw                # newest 1000 unsegmented fixes
whereami history 7d --raw --max-fixes 5000
whereami history --from 2026-05-01T00:00:00+00:00 --to 2026-05-08T00:00:00+00:00
whereami wipe --yes                      # delete stored location/AP data
whereami version                         # CLI + daemon versions
whereami --json locate                   # raw JSON output for any subcommand
whereami --timeout-secs 120 history 30d  # raise socket I/O timeout
whereami --timeout-secs 0 history 30d    # disable under an outer supervisor
whereami help
```

The CLI reaches the daemon over its unix control socket by default
(`$XDG_RUNTIME_DIR/whereamid.sock`, else `/run/whereami/whereamid.sock`).
Override with `WHEREAMI_SOCKET=<path>`, or point at an old TCP daemon with
`WHEREAMI_ADDR=<host:port>` (e.g. `127.0.0.1:4747`). See
[docs/protocol.md](../docs/protocol.md) for the access-control model.
The CLI uses the library's 40-second default. `--timeout-secs N` changes the
per-read/write timeout, while `--timeout-secs 0` disables it; use either for
long batch/history operations whose legitimate duration exceeds the default.

`wipe --yes` deletes persisted AP, pending, negative-cache, fix-history, and
last-fix rows and clears the daemon's managed in-memory location state. It does
not disable collection. Its exclusive boundary ends when the response-write
attempt terminates by success, failure, timeout, or disconnect, after which new
observations can repopulate the database. A missing complete response leaves
the result unknown; verify or retry, since wipe is idempotent. It is logical
deletion rather than forensic erasure of SQLite/WAL remnants,
allocator/process-memory remnants outside managed caches, snapshots, backups,
logs, or storage media.

## Wire protocol

See [docs/protocol.md](../docs/protocol.md) for the full request/response
schemas. The typed response structs model the fields consumed by this client;
they are compatibility-oriented rather than byte-for-byte mirrors. Newer
defaultable fields use their type default (`0`, `false`, empty, or `None`) when
an older daemon omits them, and unknown fields from a newer daemon are ignored
during typed deserialization.

Each command also has a `*_raw` method. The CLI's `--json` mode uses that raw
path: after the transport removes the framing newline and surrounding
whitespace, syntactically valid daemon JSON is forwarded without typed
re-serialization, preserving unknown fields and omission versus `null`. If the
transport fails or the daemon returns non-JSON, the CLI instead emits its own
`{"ok":false,"error":"..."}` object. Exact exit and framing behavior is
documented in [the protocol reference](../docs/protocol.md#official-cli---json-behavior).

## License

MIT.
