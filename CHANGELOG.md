# Changelog

All notable changes to whereami. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows semver.

## [0.6.0] — 2026-05-12

Three deferred follow-ups from the v0.5.0 review, landed as a small
point release. No wire-format breaking changes.

### Added

- **`whereami` CLI uses clap derive** (task-0079). `--json` and
  `--scan-time=no` are no longer hidden flags; `whereami help` honestly
  lists every flag. Legacy invocations preserved byte-for-byte
  (`whereami` → `locate`, single-letter aliases, `--scan-time=no`,
  `--from`/`--to` for history). 10 new tests pin the surface.
- **Typed `db_status` and `provenance` enums on the wire** (task-0062).
  Same JSON spelling (`api`/`cache`/`not_found`, `fresh`/`stale`/
  `not_found`/`unknown`); strict on the daemon, `#[serde(other)]` on
  the client so older clients survive future daemon additions.

### Changed

- **`get_pending` returns `Vec<(String, i32)>`** instead of the unused
  4-field `PendingAp` struct (task-0060). Pending SQL table unchanged;
  re-add a struct when a real consumer appears.

## [0.5.0] — 2026-05-10

Implementation sweep of backlog tasks 0046–0081 surfaced by the v0.4.0
four-agent code review. 13 commits across 8 phases. No breaking wire-format
changes; one schema upgrade (idempotent) carried over from 0.4.0.

### Added

- **Cooperative shutdown.** `DaemonState.shutdown` (`tokio::sync::Notify`)
  signals background loops to exit at the next iteration boundary. SIGTERM
  / SIGINT now waits up to 5 s for the scan, pending-drain, history-prune,
  and server tasks to drain before aborting (task-0075). Replaces the prior
  log-and-exit.
- **Stats observability**: `inflight` (provider lookups in progress) and
  `db_write_failures` (cumulative best-effort write failures since start)
  on the wire (tasks 0074 + 0076).
- **Apple WPS encoder/decoder cross-check.** `decode_response` now drops
  BSSIDs the server returned but we did not request, with a debug log
  (task-0049).
- **WRITE_TIMEOUT** on the daemon socket (5 s) symmetric to READ_TIMEOUT —
  slow-reading clients can no longer pin a tokio task with a buffered
  response (task-0080).
- **Wire-format reference**: `docs/protocol.md`, the canonical wire-format
  reference for all 7 commands, with client examples in bash/Python/Rust
  (task-0072).
- **whereami-client/README.md** documenting the typed Rust client library
  and the `whereami` CLI; Cargo.toml gains `readme`/`description`/`license`
  for cargo publish (task-0070).
- **CHANGELOG.md** at repo root in Keep-a-Changelog format (task-0067).
- **`whereamid::geo`** module — single canonical `haversine_m` replacing
  three duplicated copies across `trilaterate.rs`, `history.rs`, and the
  proptest suite (task-0081).
- **NotFoundPolicy enum** (`PerProvider | AtChainEnd | Never`) replaces
  the prior pair of mutually-exclusive booleans on `ChainPolicy`. Plus a
  `delete_pending_on_not_found` policy bit that collapses the now-removed
  `drain_cleanup_after_chain` (task-0052).
- **`Request` is a `#[serde(tag="cmd")]` enum** — `dispatch_command` is
  exhaustive; illegal field combos can no longer slip through unnoticed
  (task-0057). Wire format unchanged.
- **CLI `dispatch<T>` + server `ok_json<T>` helpers** eliminate ~80 LOC
  of identical match-ladder boilerplate (task-0058).
- **`From<&LastFix> for LastFixRow` + `TryFrom`** consolidate three open-
  coded marshalling sites (task-0059).
- **`DaemonState::with_db<R>(closure)` helper** centralises lock + poison-
  recovery for the common one-shot pattern (task-0078).
- **`MAX_RESOLVE_BSSIDS = 256`** caps `/resolve` request size to prevent
  self-DOS via a 7000-BSSID 64 KiB JSON (task-0053).
- **`tracing-subscriber` env-filter feature** so `RUST_LOG` actually
  controls log level (task-0054).
- **Named magic numbers**: `PENDING_DRAIN_BATCH`, `NOT_FOUND_REVIVAL_BATCH`,
  `APPLE_RESPONSE_HEADER_LEN`, `APPLE_NOT_FOUND_THRESHOLD` (task-0063).

### Changed

- **Migrations are now transactional.** `run_migration<F>(|tx| ...)` helper
  wraps each `migrate_v*_to_v*` body in an `unchecked_transaction`. On any
  Err the DDL is rolled back; the schema_version stamp commits atomically
  with the migration (task-0047). Latent v3→v4 / v4→v5 stamping bug fixed:
  each migration now stamps its own target version literal instead of the
  current `SCHEMA_VERSION` constant.
- **`last_fix` write race closed.** Both `handle_locate` and the address-
  backfill background task hold the in-memory mutex across the DB write
  (task-0046). Documented lock ordering: tokio::Mutex → std::Mutex.
- **`server.rs` and `db.rs` modularised.** `AddressCache` (+ tests + key
  math) extracted to `server/address_cache.rs`. `Source` enum (+ tests)
  extracted to `db/source.rs`. Public API surface unchanged via re-exports
  (tasks 0055 + 0056).
- **Apple WPS sentinel** named (`APPLE_NOT_FOUND_THRESHOLD = -179.0`) and
  documented; behaviour unchanged.
- **Per-BSSID resolution log** demoted from INFO to DEBUG so default-level
  journal output is bounded by scan-cycle frequency, not per-BSSID
  resolution count (task-0073).
- **README rewritten** to lead with the `whereami` CLI binary instead of
  socat. Apple WPS zero-config advertised. All 16 CLI flags listed.
  Justfile recipes surfaced. Section ordering optimised for first-time-
  user discoverability (task-0066).
- **PRD aligned to v0.4.0+ reality**: BeaconDB-as-runtime references gone;
  scanner section reflects nmcli-primary; future-work no longer claims
  shipped features; CLI args block complete; Data Model shows schema v5
  with `last_fix` and `fixes` tables (task-0068).
- **Architecture diagrams** in README and PRD are now ASCII text-art
  that cannot drift silently (task-0069).
- **Stats handler** uses `with_db` and emits inflight + db_write_failures
  alongside existing fields.
- **`AddressCache.get` debug-asserts** lat/lon are finite; saturation
  semantics on `f64 as i32` documented (task-0064).
- **WigleClient/AppleClient/NominatimClient/AddressCache** test-default
  constructors marked `#[allow(dead_code)]` so production paths must use
  the `with_timeout` / `with_ttl_days` variants explicitly. Removed
  `MockProvider::calls` (zombie no-caller) (task-0061 partial).

### Fixed

- **`parse_range` panic on non-ASCII**: `split_at(spec.len() - 1)` was
  byte-indexed; `"7日"` panicked on a non-char-boundary. Now uses the
  last char's UTF-8 length (task-0048).
- **Synthesized -90 dBm fallbacks** in `handle_locate` candidate ranking
  and `handle_debug` removed. Stable BSSIDs absent from the latest scan
  no longer feed fake readings into trilateration weights or fake values
  into debug output. `DebugBssid.signal_dbm` is now `Option<i32>`
  (wire-format change for the debug command only) (task-0051).
- **Nominatim empty `display_name`** now returns Err instead of caching
  an empty string as the address (task-0050).
- **Metadata garbage values** (e.g. someone hand-pokes the SQLite file)
  now log a warn via `parse_api_calls_value` instead of silently resetting
  the daily counter to 0 (task-0077).

### Removed

- **`drain_cleanup_after_chain`** — collapsed into `resolve_chain` via
  the `delete_pending_on_not_found` policy bit (task-0052).
- **Three duplicated `haversine_m` copies** (trilaterate.rs, history.rs,
  proptests.rs) — replaced with re-imports from `whereamid::geo`
  (task-0081).
- **`backlog/email-preferences.json`** untracked + gitignored. Per-user
  backlog tooling artefact that should never enter the repo (task-0071).

### Deferred

Three tasks remain in the To Do queue with documented rationale:
- **task-0060** trim PendingAp's unused fields — touches SQL + consumers;
  deferred until PendingAp gains a new consumer.
- **task-0062** wire-format enums for `db_status` and resolve provenance —
  current 4/3-value string vocabularies are stable; conversion would
  require backwards-compat shims for older clients.
- **task-0079** CLI clap migration — full conversion touches every
  subcommand for marginal UX gain; deferred until adding the next
  subcommand makes argv branches painful.

### Schema migration

No new schema changes in 0.5.0. v3→v4 (last_fix) and v4→v5 (fixes
timeseries) carried over from 0.4.0 are now transactional and idempotent
(task-0047). Existing v3 databases auto-migrate forward on first 0.5.0
startup.

### Tests

134 tests at release tip (50 lib unit + 75 bin unit + 9 proptest), up
from 130 at the start of the 0.5.0 sweep. New: migration-rollback,
parse_range non-ASCII, address_cache TTL/eviction, geo round-trip and
reference distance, two drain_chain end-to-end policy tests.

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

Test count grew from 57 (at v0.3.0) to **134** (50 lib unit + 75 bin unit
+ 9 proptest at v0.4.0 release-tip). Highlights:
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
