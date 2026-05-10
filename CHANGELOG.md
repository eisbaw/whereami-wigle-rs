# Changelog

All notable changes to whereami. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows semver.

## [0.4.0] — 2026-05-10

A substantial release: 50+ commits implementing 30+ backlog tasks.
Focuses on correctness, robustness, observability, and developer ergonomics.

### Added

- **Apple WPS** is the primary geolocation backend. Free, no API key required.
  WiGLE remains the (optional) secondary.
- **`whereami` CLI binary** (`cargo run --bin whereami`) replaces ad-hoc
  `socat` invocations. Subcommands: `locate`, `scan`, `resolve`, `stats`,
  `debug`, `version`, `history`, `help`. Supports `--json` for raw output.
- **Location history** (`whereami history <range>`). Every successful fix is
  recorded in a SQLite timeseries; the command groups consecutive fixes into
  stay-point segments via a spherical-mean centroid + distance threshold +
  minimum-duration filter. Range syntax: `7d`, `24h`, `30m`, `1w`, `300s`,
  or `--from`/`--to` RFC3339 timestamps.
- **`last_fix` persistence**. The most recent fix is stored in a single-row
  `last_fix` table and rehydrated on daemon startup, so `locate` answers
  immediately after a restart instead of waiting for a fresh resolve cycle.
- **Provider trait + `resolve_chain` orchestrator**. The four prior
  duplicates of the Apple→WiGLE cascade collapsed into a single function
  parameterized by a small `ChainPolicy`. Adding the next provider is one
  enum variant + one match arm.
- **Atomic daily quota**. WiGLE quota check-and-reserve runs in a single
  SQL transaction. No more overshoot under concurrent locates.
- **In-flight dedup**. Concurrent scan-loop / locate-cold-start / pending-drain
  spawns coalesce to one provider call per BSSID via an `Arc<Mutex<HashSet>>`.
- **Source-priority ladder**. `aps.source_priority` column ensures Apple
  fixes (priority 40) aren't overwritten by later WiGLE fetches (30) for
  the same BSSID. Migration backfills existing rows.
- **Schema v5**: `last_fix` (single-row CHECK), `fixes` (timeseries), all
  migrations now run in transactions and roll back atomically on failure.
- **Address-cache TTL**. Reverse-geocoded addresses cached on a ~10m grid
  and re-resolved after `--address-cache-ttl-days` (default 7).
- **Configurable HTTP timeouts**: `--http-timeout-secs` (Apple/WiGLE,
  default 15), `--nominatim-timeout-secs` (default 30). Connect timeout
  is fixed at 5s.
- **Wi-Fi 6E (6 GHz) channel mapping** in scanner.
- **SIGTERM cooperative shutdown**. Background loops (scan, pending drain,
  history prune) exit at the next iteration boundary; main awaits with a
  5s drain timeout. Replaces the prior log-and-exit.
- **`Args::validate()`** runs before any side effects. Bad combinations
  (e.g. `--debounce-threshold 100 --debounce-window 5`) produce a clean
  error instead of crashing inside `Debouncer::new`.
- **Stats observability**: `inflight` (provider lookups in progress) and
  `db_write_failures` (cumulative best-effort write failures) on the wire.
- **Justfile** with `build`, `test`, `lint`, `fmt`, `e2e`, `qa`, `fuzz`,
  `fuzz-all`, and per-target fuzz recipes. `just` is in the dev shell.
- **`cargo-fuzz` harness** with five targets: iw parser, nmcli parser,
  Apple decoder, Apple encoder, trilateration. Devshell exports
  `LD_LIBRARY_PATH` so `cargo fuzz run` works without manual setup.
- **`docs/protocol.md`** (canonical wire format reference).
- **`whereami-client` library** with typed response structs (`LocateResponse`,
  `HistoryResponse`, etc.) and a `DaemonResponse` trait for uniform
  ok/error handling.
- **`whereamid::geo`** module with the canonical `haversine_m`.
- **NixOS `homeManagerModules.default`** for per-user installations on
  non-NixOS systems, alongside the existing `nixosModules.default`.

### Changed

- **Trilateration math**: spherical mean (3D unit-vector centroid) instead
  of arithmetic lat/lon mean. Eliminates the antimeridian bug (two APs at
  lon=±179 used to centroid to lon=0) and behaves correctly near the poles.
- **Apple WPS protobuf length** is a 4-byte big-endian u32 (was a u8 with
  silent truncation past 255 bytes; the Python references share this bug).
- **Apple WPS decoder** cross-checks returned BSSIDs against the requested
  set; strays are dropped with a debug log instead of being cached.
- **Nominatim** rejects empty `display_name`; the address cache no longer
  stores empty strings.
- **nmcli signal field**: rows with empty/malformed signal are skipped
  instead of being coerced to a fake -90 dBm. Also skipped in trilateration
  candidate ranking (`server.rs` cold-start path).
- **`tracing-subscriber`** built with the `env-filter` feature; `RUST_LOG`
  now actually controls log level.
- **Per-BSSID resolution log** demoted from INFO to DEBUG so default
  journal output is bounded by scan-cycle frequency.
- **Request shape** is now a `#[serde(tag="cmd")]` enum; old wire format
  unchanged. `dispatch_command` is exhaustive — illegal field combinations
  on a given command are no longer silently accepted.
- **`/resolve`** rejects requests with more than 256 BSSIDs (self-DOS guard).
- **`parse_range`** is UTF-8-safe; non-ASCII input returns Err instead of
  panicking on a non-char-boundary `split_at`.
- **`AddressCache.get`** checks TTL on read; expired entries report as miss.
- **CLI** uses typed response structs end-to-end. Six near-identical
  match ladders collapsed into a single `dispatch<T>` helper.

### Fixed

- **`last_fix` write race**: the address-backfill background task and
  `handle_locate` no longer drop the in-memory mutex before the DB write,
  closing a window where stale lat/lon could overwrite a newer on-disk row.
- **Latent partial-migration bug**: `migrate_v3_to_v4` and `migrate_v4_to_v5`
  used to stamp `SCHEMA_VERSION` (= the latest constant) instead of their
  own target version. A daemon killed mid-chain on first startup could
  claim version=5 without having created the `fixes` table. Each migration
  now stamps its own target literal.
- **`-90 dBm` fallbacks** removed from `handle_locate` candidate ranking
  and `handle_debug`. Stable BSSIDs without a current scan signal no longer
  feed fake readings into trilateration weights.
- **Resolver `is_pending` redundancy**: the `mark_not_found_at_chain_end`
  branch no longer skips the not_found mark when the BSSID is already
  pending. The `transient_error` guard already covers the "we just queued
  it" case; the `is_pending` check made task-0045 a no-op for
  `drain_once`.

### Removed

- **`BeaconDbClient`** (was constructed but never called). The
  `Source::BeaconDb` enum variant remains as read-only legacy so historical
  DB rows still load at the correct priority.
- **Six clippy errors** (`unnecessary_sort_by`) that were blocking
  `just e2e` under `-D warnings`.
- **Provider cascade duplication**: the four near-identical Apple→WiGLE
  loops in `resolve_for_locate`, `resolve_readonly`, `resolve_background`,
  and `pending::drain_once` collapsed into a single `resolve_chain`.
- **`drain_cleanup_after_chain`**: collapsed into `resolve_chain` via the
  `delete_pending_on_not_found` policy bit.

### Schema migration

Existing v3 databases (whereami 0.3.x) auto-migrate forward to v5 on
first 0.4.0 startup:
- v3→v4 adds the `last_fix` table.
- v4→v5 adds the `fixes` table + index for location history.
Both run inside transactions and roll back on failure. Downgrade to 0.3.x
is not supported (the binary will warn that the schema is too new).

### Tests

Test count grew from 57 (at v0.3.0) to **146** (50 lib unit + 75 bin unit
+ 9 proptest at v0.4.0 release-tip plus the new geo tests). Highlights:
- Atomic rate-limit: 64-thread concurrency test asserting reservations ≤ limit.
- Migration rollback: failing body leaves DB at prior version.
- Antimeridian / polar / antipodal trilateration cases.
- Provider chain tests (cache hit, first-found, both-NotFound, transient
  error guard, in-flight dedup) using a `cfg(test)`-only `Provider::Mock`.
- Apple WPS encoder fuzz target asserting the u32 BE payload-length
  invariant for inputs up to 1024 BSSIDs.

## [0.3.0] — 2026-05

Initial public release. Apple WPS via reverse-engineered ARPC protocol,
WiGLE secondary, SQLite cache, debounce, weighted-centroid trilateration,
JSON-lines TCP server. See [task-0001 through task-0018] in `backlog/tasks/`.
