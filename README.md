# whereami

Wi-Fi geolocation daemon for Linux. Scans nearby access points, resolves their
positions via Apple WPS (free, no auth) with WiGLE as a secondary backend,
caches results in SQLite, and answers "where am I?" over an access-controlled
Unix socket.

```
$ whereami locate
Magstræde 5, 1204 København K, Denmark
55.707250, 12.585598  ±12m  (6 sources)
```

## Why

Browser-based geolocation (Google, Mozilla) sends your Wi-Fi environment to
third-party servers on every query. GeoClue's Mozilla backend is dead.
whereami runs locally: it answers offline once your area is cached, talks to
Apple WPS on demand without an API key, and the cache lives on your disk.

## Quick start

```bash
nix-shell                                     # or: nix develop (same dev shell)
just build                                    # cargo build via Justfile
cargo run --bin whereamid -- --interface $(ip -o link | awk -F': ' '/state UP/ && /wl/ {print $2; exit}')
# in another terminal:
cargo run --bin whereami -- locate
```

That's it. **Apple WPS works without any credentials.** WiGLE is optional and
adds coverage; `wigle.net` gives a free key.

## CLI usage

The `whereami` binary speaks the daemon's socket protocol so you don't have to.

```bash
whereami locate              # where am I?
whereami scan                # currently visible Wi-Fi networks
whereami stats               # cache size, API quota, in-flight count
whereami debug               # per-BSSID debounce + DB classification
whereami history 24h         # stay-point segments over the last 24 hours
whereami history 7d          # last 7 days; supports s/m/h/d/w
whereami history --from 2026-05-01T00:00:00+00:00 --to 2026-05-08T00:00:00+00:00
whereami wipe --yes          # delete stored location and observed Wi-Fi data
whereami version             # CLI + daemon versions
whereami --json locate       # raw JSON output (any subcommand)
whereami help
```

## Configuration

The daemon's secret-store is a TOML file (default `~/.config/whereami/config.toml`):

```toml
[wigle]
api_user = "AID..."   # optional; daemon runs without WiGLE creds
api_key  = "..."
```

Keep this file owner-only because another local user who can read or modify it
can steal or replace the WiGLE API credentials:

```bash
chmod 600 ~/.config/whereami/config.toml
```

At startup, the daemon warns when any group or world permission bit is set. It
does not print credential values, change the mode, or reject the file; fix the
permissions explicitly. Read-only owner access (`0400`) is also safe.

Everything else is a CLI flag (see `cargo run --bin whereamid -- --help`):

```
--socket <PATH>                     Unix control socket [$XDG_RUNTIME_DIR/whereamid.sock]
--bind <ADDR>                       TCP bind address (opt-in, unauthenticated; off by default)
--allow-nonlocal-bind               Permit a non-loopback --bind (exposes the daemon)
--db <PATH>                         SQLite database [/var/lib/whereami/aps.sqlite]
--interface <NAME>                  Wi-Fi interface [wlan0]
--config <PATH>                     Config file [~/.config/whereami/config.toml]
--max-connections <N>               Shared active connection cap, 1..=4096 [32]
--scan-interval-fast <S>            Fast-phase scan interval [10]
--scan-fast-duration <S>            Fast phase duration [60]
--scan-interval-slow <S>            Steady-state scan interval [60]
--debounce-window <N>               Ring buffer size [10]
--debounce-threshold <N>            Min appearances to be stable [5]
--top-n <N>                         Strongest APs for trilateration [10]
--pending-interval <S>              Pending queue drain interval [300]
--pending-max-attempts <N>          Give up after N failures [20]
--daily-limit <N>                   Max WiGLE API calls/day [100]
--not-found-ttl-days <N>            Re-check not-found after N days [30]
--address-approx                    Include street address (OSM Nominatim)
--address-cache-ttl-days <N>        Reverse-geocode cache TTL [7]
--http-timeout-secs <N>             Apple/WiGLE total timeout [15]
--nominatim-timeout-secs <N>        Nominatim total timeout [30]
--history-retention-days <N>        Prune fixes older than N days [30]
--aps-retention-days <N>            Prune APs not seen for N days [30]
--history-segment-distance-m <M>    Stay-point clustering threshold [100]
--history-segment-min-duration-secs Min stay-point duration [300]
```

## How it works

```
                 unix socket (0600/0660)
  any client  ------>  whereamid  ----> local SQLite cache
  (whereami CLI,         |                    |
   curl, Python…)        |              cache miss?
                         |                    |
                         +---------> Apple WPS  (no auth, primary)
                                     WiGLE API  (secondary, optional)
                                     Nominatim  (reverse geocode, optional)
```

1. Background scanner picks up nearby Wi-Fi APs via `nmcli` (preferred) or `iw`.
2. Debounce filter ensures only stable APs (seen repeatedly) drive the cache.
3. Positions resolve via Apple WPS first, then WiGLE; results cache locally.
4. Outlier rejection drops APs with stale/wrong positions.
5. Spherical-mean trilateration computes your location (antimeridian-safe).
6. Optional reverse geocoding via OSM Nominatim adds a street address.
7. Every successful fix is recorded in a `fixes` timeseries; the `history`
   command groups them into stay-point segments.

## Accuracy

Urban areas with good Apple/WiGLE coverage: ~10-30m with 5+ resolved APs.
Suburban: 50-100m. Rural areas with sparse coverage may not resolve at all.

## Caching and offline operation

Apple WPS is free; the daemon will hit it on every cache miss. WiGLE has a
daily quota the daemon respects atomically (no quota overshoot under
concurrent locates). Once your area is cached, all queries are answered
locally with zero network traffic.

When offline, unknown APs are queued in a `pending` table. A background
drain re-tries them when connectivity returns, so the cache fills in
opportunistically.

After a daemon restart, the most recent fix is rehydrated from disk — `whereami
locate` answers immediately while the next scan-and-resolve cycle runs.

## Privacy, retention, and wiping

By default, the daemon removes location-history fixes older than 30 days and
resolved AP rows that have not been observed for 30 days. It runs one sweep at
startup and then daily, alternating bounded 1,000-row transactions between the
two tables. Configure the windows with `--history-retention-days` and
`--aps-retention-days`, or the Nix options `historyRetentionDays` and
`apsRetentionDays`. A value of `0` keeps that table indefinitely.

To delete the daemon's current logical location data immediately:

```bash
whereami wipe --yes
```

The command uses the filesystem-permission-gated Unix control socket and is
always refused over unauthenticated TCP, including loopback TCP. It atomically
deletes resolved APs, pending APs, negative-cache entries, fix history, the
persisted last-known fix, and the fix-ID sequence. It also clears the daemon's
managed in-memory scan/debounce, last-fix, address-cache, geocode-queue, and
in-flight lookup state. The success response reports exact per-table counts.
Schema state and operational metadata such as API quota and shutdown counters
are intentionally retained. Daemon configuration and WiGLE credentials live
outside these tables and are untouched.

Socket access is the authorization boundary. Every user authorized by the
socket's filesystem permissions (including every NixOS `allowedUsers` member)
can run the destructive wipe; there is no read-only or per-command role. The
`--yes` confirmation prevents accidental invocation but grants no additional
authority.

A wipe does not disable collection. Its exclusive boundary ends when the
server's response-write attempt terminates, whether by success, write failure,
timeout, or disconnect; later scans and queries may then repopulate the
database. If no complete response arrives, completion is unknown: verify the
state or retry the idempotent wipe. Stop or disable the daemon as well if
collection must remain off. This is logical live-state deletion, not forensic
erasure: SQLite free pages or WAL remnants, allocator/process-memory remnants
outside the managed structures above, filesystem snapshots, backups, storage
media, logs, and provider traffic already sent are outside its scope. Use
encrypted storage and separately manage snapshots/backups when those threats
matter.

An internal invariant panic after the database commit is fail-stop: the daemon
keeps the publication boundary held and exits nonzero before it can resume
serving partially cleared state. A systemd service can restart against the
already-wiped database; clients should treat the interrupted response as
unknown and verify or retry.

## Development

```bash
nix-shell            # or: nix develop; both select the flake's default dev shell
just build           # cargo build
just test            # cargo test --workspace
just lint            # cargo clippy --all-targets -- -D warnings
just fmt             # cargo fmt --check
just e2e             # shell.nix evaluation + test + lint (pre-commit gate)
just qa              # e2e + fmt + 15s fuzz on every target
just fuzz            # quick smoke (15s/target)
just fuzz-all        # long smoke (60s/target)
just fuzz-apple-encode   # focused: Apple WPS encoder fuzz
```

`shell.nix` is a compatibility entry point for tools that expect `nix-shell`;
it selects the same flake `devShells.<system>.default` used by `nix develop`,
so the dependency list has one source of truth. The dev shell exposes `just`,
the nightly Rust toolchain, `cargo-fuzz`, and all native deps.
`RUST_LOG=whereamid=debug` enables verbose logging.

The documented development workflow requires a Git checkout and a
flakes-enabled Nix for both entry points. The `git+file`/`getFlake` shim does
not support source tarballs, Nix store paths, or older flakes-disabled Nix.

### Formal verification (Kani)

The parser-heavy code paths (Apple WPS protobuf decoder, scanner string
parsers, history range parser) carry `#[cfg(kani)]`-gated proof harnesses
that assert panic-freedom over a bounded input domain, intended to be
discharged by the [Kani](https://github.com/model-checking/kani) bounded
model checker. Where they prove, they cover the whole bounded domain
exhaustively rather than sampling it like fuzzing does. Running them
requires the out-of-band setup below; they are not part of `just e2e` or
CI yet, so treat a harness as "written, not yet proven" until you have
run `just kani` locally.

Kani brings its own forked Rust toolchain, so it lives **outside** the
Nix dev shell. One-time setup:

```bash
cargo install --locked kani-verifier
cargo kani setup
```

Then:

```bash
just kani                                       # run all proofs
cd whereamid && cargo kani --harness <name>     # single proof
```

The harnesses are gated on `cfg(kani)` so normal `cargo build`,
`cargo test`, and `cargo clippy` are unaffected. The `geo::haversine_m`
harness reasons over `f64`; if it doesn't terminate in reasonable time
on your hardware, comment it out — the same surface is exercised
stochastically by the proptest suite.

## NixOS module

```nix
{
  imports = [ inputs.whereami.nixosModules.default ];
  services.whereami = {
    enable = true;
    wifiInterface = "wlp9s0f0";
    credentialsFile = "/run/secrets/whereami.toml";
    addressApprox = true;
  };
}
```

The systemd service runs with `CAP_NET_ADMIN`, `DynamicUser=true`, and
hardened sandboxing. The flake also exposes `homeManagerModules.default` for
per-user installations on non-NixOS systems.

## Protocol

For language-agnostic clients: see [docs/protocol.md](docs/protocol.md). The
wire format is JSON-lines on a one-shot connection over a Unix domain socket
(access-controlled by filesystem permissions; opt-in TCP via `--bind`). Any
language with a unix-socket client can talk to the daemon.

## Architecture

See [docs/adr/](docs/adr/) for Architecture Decision Records covering all
major design choices. Highlights:

- TCP JSON-lines, not HTTP (any language can be a client).
- `nmcli` for scanning, `iw` as fallback (no root needed for `nmcli`).
- Debounce filter prevents transient APs from polluting the cache.
- Spherical-mean trilateration (great-circle math, antimeridian-safe).
- Adaptive outlier rejection via median distance.
- Source-priority ladder so Apple-resolved fixes don't get overwritten by
  later WiGLE fetches for the same BSSID.
- In-flight dedup so concurrent `locate` + scan-loop spawns coalesce to one
  Apple/WiGLE call per BSSID.
- Atomic daily-quota reservation (single SQL transaction).
- CLI args for operations, TOML file for secrets only.

### Dataflow diagrams

Four focused diagrams (one per pipeline) — sources in `docs/*.drawio`,
rendered to SVG. Click for full size.

**Scan & Resolve** — Wi-Fi scan → debouncer → resolver chain → providers → SQLite writes:

[![Scan & Resolve dataflow](docs/dataflow-scan.svg)](docs/dataflow-scan.svg)

**Locate** — TCP request → cached AP lookup → outlier filter → trilateration → response:

[![Locate dataflow](docs/dataflow-locate.svg)](docs/dataflow-locate.svg)

**History** — stay-point segmentation over the fixes timeseries:

[![History dataflow](docs/dataflow-history.svg)](docs/dataflow-history.svg)

**Persist** — SQLite (WAL) topology, showing who writes and who reads each table:

[![Persist topology](docs/dataflow-persist.svg)](docs/dataflow-persist.svg)

See `CHANGELOG.md` for what changed since v0.3.0.

## License

MIT
