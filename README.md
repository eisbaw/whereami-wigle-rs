# whereami

Wi-Fi geolocation daemon for Linux. Scans nearby access points, resolves their
positions via Apple WPS (free, no auth) with WiGLE as a secondary backend,
caches results in SQLite, and answers "where am I?" over a localhost TCP
socket.

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
nix develop                                   # or: nix-shell
just build                                    # cargo build via Justfile
cargo run --bin whereamid -- --interface $(ip -o link | awk -F': ' '/state UP/ && /wl/ {print $2; exit}')
# in another terminal:
cargo run --bin whereami -- locate
```

That's it. **Apple WPS works without any credentials.** WiGLE is optional and
adds coverage; `wigle.net` gives a free key.

## CLI usage

The `whereami` binary speaks the daemon's TCP protocol so you don't have to.

```bash
whereami locate              # where am I?
whereami scan                # currently visible Wi-Fi networks
whereami stats               # cache size, API quota, in-flight count
whereami debug               # per-BSSID debounce + DB classification
whereami history 24h         # stay-point segments over the last 24 hours
whereami history 7d          # last 7 days; supports s/m/h/d/w
whereami history --from 2026-05-01T00:00:00+00:00 --to 2026-05-08T00:00:00+00:00
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

Everything else is a CLI flag (see `cargo run --bin whereamid -- --help`):

```
--bind <ADDR>                       TCP bind address [127.0.0.1:4747]
--db <PATH>                         SQLite database [/var/lib/whereami/aps.sqlite]
--interface <NAME>                  Wi-Fi interface [wlan0]
--config <PATH>                     Config file [~/.config/whereami/config.toml]
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
--history-segment-distance-m <M>    Stay-point clustering threshold [100]
--history-segment-min-duration-secs Min stay-point duration [300]
```

## How it works

```
                    TCP :4747
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

## Development

```bash
just build           # cargo build
just test            # cargo test --workspace
just lint            # cargo clippy --all-targets -- -D warnings
just fmt             # cargo fmt --check
just e2e             # test + lint (pre-commit gate)
just qa              # e2e + fmt + 15s fuzz on every target
just fuzz            # quick smoke (15s/target)
just fuzz-all        # long smoke (60s/target)
just fuzz-apple-encode   # focused: Apple WPS encoder fuzz
```

The dev shell exposes `just`, the nightly Rust toolchain, `cargo-fuzz`, and
all native deps. `RUST_LOG=whereamid=debug` enables verbose logging.

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
wire format is JSON-lines on a one-shot TCP connection. Any language with a
TCP socket can talk to the daemon.

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

See `CHANGELOG.md` for what changed since v0.3.0.

## License

MIT
