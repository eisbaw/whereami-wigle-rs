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
use whereami_client::WhereAmIClient;

// Resolve the daemon from the environment: WHEREAMI_SOCKET / WHEREAMI_ADDR /
// the default unix-socket paths ($XDG_RUNTIME_DIR/whereamid.sock, then
// /run/whereami/whereamid.sock).
let client = WhereAmIClient::from_env()?;

// Where am I?
let resp = client.locate()?;
println!("{:.6}, {:.6}  ±{}m", resp.lat, resp.lon, resp.accuracy_m as i64);

// History over the last 24 hours.
let h = client.history(Some("24h".into()), None, None)?;
for seg in &h.segments {
    println!("{} -> {}  {} fixes", seg.start_rfc3339, seg.end_rfc3339, seg.n_fixes);
}

// Pin a specific transport instead of resolving from the environment:
let via_socket = WhereAmIClient::unix("/run/whereami/whereamid.sock");
let via_tcp = WhereAmIClient::tcp("10.0.0.5:4747"); // opt-in, unauthenticated
let stats = via_socket.stats()?;
```

All daemon responses share an `ok: bool` flag and an `error: Option<String>`
field. The crate exposes a `DaemonResponse` trait so generic dispatchers can
inspect both uniformly:

```rust
fn handle<T: serde::Serialize + whereami_client::DaemonResponse>(r: &T) {
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
whereami history --from 2026-05-01T00:00:00+00:00 --to 2026-05-08T00:00:00+00:00
whereami version                         # CLI + daemon versions
whereami --json locate                   # raw JSON output for any subcommand
whereami help
```

The CLI reaches the daemon over its unix control socket by default
(`$XDG_RUNTIME_DIR/whereamid.sock`, else `/run/whereami/whereamid.sock`).
Override with `WHEREAMI_SOCKET=<path>`, or point at an old TCP daemon with
`WHEREAMI_ADDR=<host:port>` (e.g. `127.0.0.1:4747`). See
[docs/protocol.md](../docs/protocol.md) for the access-control model.

## Wire protocol

See [docs/protocol.md](../docs/protocol.md) for the full request/response
schemas. The crate's response structs are 1:1 mirrors of those JSON shapes.

## License

MIT.
