# whereami

Wi-Fi geolocation daemon for Linux. Scans nearby access points, resolves their positions via the WiGLE API, caches results in SQLite, and answers "where am I?" over a TCP socket.

## How it works

1. Background scanner picks up nearby Wi-Fi APs (via nmcli or iw)
2. Debounce filter ensures only stable APs (seen repeatedly) are trusted
3. Positions are resolved via WiGLE and cached locally
4. Outlier rejection drops APs with stale/wrong positions
5. Weighted centroid trilateration computes your location
6. Optional reverse geocoding via OSM Nominatim adds a street address

Once the cache is warm for your area, the daemon works fully offline.

## Quick start

```bash
# Enter dev shell
nix develop  # or: nix-shell

# Build
cargo build

# Configure WiGLE credentials (get a free account at wigle.net)
mkdir -p ~/.config/whereami
cat > ~/.config/whereami/config.toml << EOF
[wigle]
api_user = "AID..."
api_key = "..."

[beacondb]
enabled = true
EOF
chmod 600 ~/.config/whereami/config.toml

# Run (adjust interface name)
cargo run --bin whereamid -- --interface wlp9s0f0

# Query (from another terminal)
echo '{"cmd":"locate"}' | socat - TCP:127.0.0.1:4747
```

## Protocol

TCP JSON-lines on `127.0.0.1:4747`. One-shot connections: connect, send one JSON line, read one JSON response, close.

### Commands

**locate** — where am I?
```json
{"cmd":"locate"}
→ {"ok":true,"v":1,"lat":48.857,"lon":2.351,"accuracy_m":10.0,"sources":7,"cached":7,"fetched":0,"pending":0,"visible":11,"stable":9}
```

**scan** — what APs are visible?
```json
{"cmd":"scan"}
→ {"ok":true,"v":1,"networks":[{"bssid":"AA:BB:CC:DD:EE:FF","ssid":"MyWiFi","signal_dbm":-65,"channel":6}],"scan_age_ms":3200}
```

**resolve** — look up specific BSSIDs
```json
{"cmd":"resolve","bssids":["AA:BB:CC:DD:EE:FF"]}
→ {"ok":true,"v":1,"results":[{"bssid":"AA:BB:CC:DD:EE:FF","lat":48.857,"lon":2.351,"ssid":"MyWiFi","source":"cache"}]}
```

**stats** — cache statistics
```json
{"cmd":"stats"}
→ {"ok":true,"v":1,"cached_aps":42,"pending_aps":0,"not_found_aps":3,"db_size_bytes":49152,"api_calls_today":5}
```

## CLI options

```
whereamid [OPTIONS]

--bind <ADDR>              TCP bind address [default: 127.0.0.1:4747]
--db <PATH>                SQLite database path [default: /var/lib/whereami/aps.sqlite]
--interface <NAME>         Wi-Fi interface [default: wlan0]
--config <PATH>            TOML config file [default: ~/.config/whereami/config.toml]
--scan-interval-fast <S>   Scan interval, fast phase [default: 10]
--scan-fast-duration <S>   Fast phase duration [default: 60]
--scan-interval-slow <S>   Scan interval, steady state [default: 60]
--debounce-window <N>      Ring buffer size [default: 10]
--debounce-threshold <N>   Min appearances to be stable [default: 5]
--top-n <N>                Strongest APs for trilateration [default: 10]
--pending-interval <S>     Pending queue drain interval [default: 300]
--pending-max-attempts <N> Give up after N failures [default: 20]
--daily-limit <N>          Max WiGLE API calls/day [default: 100]
--not-found-ttl-days <N>   Re-check not-found after N days [default: 30]
--address-approx           Include street address via OSM Nominatim
```

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

The systemd service runs with `CAP_NET_ADMIN`, `DynamicUser=true`, and hardened sandboxing.

## Architecture

See [docs/adr/](docs/adr/) for Architecture Decision Records covering all major design choices.

Key design decisions:
- TCP JSON-lines, not HTTP (any language can be a client)
- nmcli for scanning (no root needed)
- Debounce filter prevents transient APs from polluting the cache
- Adaptive outlier rejection via median (handles moved routers)
- Cache-first with negative cache and pending queue for offline resilience
- CLI args for operations, TOML file for secrets only

## Testing

```bash
cargo test                         # 24 unit + 9 property tests
PROPTEST_CASES=10000 cargo test    # extended fuzzing
```

## License

MIT
