//! Control-channel server: JSON-lines protocol, one-shot connections.
//!
//! The canonical transport is a Unix domain socket, gated by filesystem
//! permissions (the Unix-native access-control model — see ADR-003). TCP is an
//! opt-in, unauthenticated secondary transport (`--bind`). Both share one
//! accept loop and identical timeouts/limits; `handle_connection` is generic
//! over the stream type. task-0196.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixListener};
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

use crate::apple::AppleClient;
use crate::config::Args;
use crate::db::{DbHandle, DebugDbStatus};
use crate::debounce::Debouncer;
use crate::nominatim::NominatimClient;
use crate::wigle::WigleClient;

const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Total budget to write the response and close the socket. A slow client
/// reading slowly should not pin a tokio task with the response buffered
/// indefinitely. Symmetric to READ_TIMEOUT in magnitude (task-0080).
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST_BYTES: u64 = 64 * 1024; // 64 KiB max request size

/// Maximum number of BSSIDs accepted in a single `resolve` request.
/// MAX_REQUEST_BYTES bounds the wire size, but a 64 KiB JSON of 6-byte
/// BSSIDs is ~7000 entries; resolve_chain enqueues DB work per BSSID,
/// so an unbounded count would let one hostile request starve the rest
/// of the daemon. task-0053.
const MAX_RESOLVE_BSSIDS: usize = 256;
const WIPE_CONFIRMATION: &str = "delete-all-location-data";
const WIPE_CONFIRMATION_ERROR: &str = "wipe: confirmation must equal 'delete-all-location-data'";
const WIPE_TCP_ERROR: &str =
    "wipe: unavailable over unauthenticated TCP; use the Unix control socket";
const WIPE_BUSY_ERROR: &str = "wipe: busy; retry later";
const WIPE_RESULT_UNKNOWN_ERROR: &str = "wipe: result unavailable; verify and retry";
const STALE_AFTER_WIPE_ERROR: &str = "request invalidated by data wipe; retry";
#[cfg(test)]
const WIPE_GUARD_DROP_MARKER_ENV: &str = "WHEREAMID_WIPE_GUARD_DROP_MARKER";

// task-0056: AddressCache extracted to server/address_cache.rs. Public
// re-export so existing call sites (`crate::server::AddressCache`) keep
// working.
mod address_cache;
use address_cache::address_cache_key;
pub use address_cache::AddressCache;
pub(crate) mod geocode;

/// Direct synchronous access exists only for legacy unit-test setup and
/// assertions. Production code has no path to a rusqlite connection.
#[cfg(test)]
pub fn lock_db(state: &DaemonState) -> std::sync::MutexGuard<'_, crate::db::Database> {
    state.test_db.lock().unwrap_or_else(|e| {
        tracing::error!("DB mutex was poisoned (a thread panicked while holding it) — recovering");
        e.into_inner()
    })
}

impl DaemonState {
    /// Bump the cumulative best-effort-DB-write-failure counter.
    /// task-0076. Use at every warn-and-continue site that follows a
    /// failed database operation so silent corruption is visible via
    /// the stats endpoint.
    pub fn record_db_failure(&self) {
        self.db_write_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Spawn work tracked by the shutdown drain (task-0102). Background
    /// resolves and accepted connection handlers route through here so a
    /// SIGTERM can wait under one budget and force-abort stragglers before the
    /// DB actors stop. Nominatim is a separate named one-worker loop
    /// (task-0193), never a tracked spawn.
    ///
    /// Refuses to spawn once shutdown has been signalled — this structurally
    /// enforces "no new work after shutdown began" (task-0190 AC#2); the future
    /// is dropped, not run. Completed tasks are reaped on each call so the
    /// JoinSet does not accumulate finished-task slots (a JoinSet only frees a
    /// slot when the task is joined).
    ///
    /// Per-connection handlers are tracked so database actors cannot stop
    /// while a handler still owns/enqueues through `DbHandle`; connection
    /// permits are captured by these futures and are returned on refusal or
    /// force-abort (task-0201).
    pub fn spawn_tracked<F>(&self, fut: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.spawn_tracked_after_precheck(fut, || {});
    }

    fn spawn_tracked_after_precheck<F, H>(&self, fut: F, after_precheck: H)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
        H: FnOnce(),
    {
        if self.shutdown.is_cancelled() {
            debug!("spawn_tracked refused: shutdown in progress");
            return;
        }
        after_precheck();
        let mut set = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        // This check and main's mem::take use the same mutex. A spawn that
        // passed the cheap check before cancellation but lost the lock race
        // cannot insert into the replacement JoinSet after shutdown took the
        // old one for draining.
        if self.shutdown.is_cancelled() {
            debug!("spawn_tracked refused: shutdown began while waiting for task ownership");
            return;
        }
        while set.try_join_next().is_some() {}
        set.spawn(fut);
    }
}

/// Shared daemon state, accessible from connection handlers.
/// SQLite is owned by dedicated reader/writer OS threads behind `DbHandle`;
/// Tokio workers only enqueue commands and await oneshot results.
pub struct DaemonState {
    pub db: DbHandle,
    #[cfg(test)]
    pub _db_runtime: Option<crate::db::DbRuntime>,
    #[cfg(test)]
    pub test_db: std::sync::Mutex<crate::db::Database>,
    pub debouncer: tokio::sync::Mutex<Debouncer>,
    /// Linearization boundary shared by every privacy-bearing publisher and
    /// the destructive wipe command.
    pub(crate) privacy: crate::privacy::PrivacyCoordinator,
    pub args: Args,
    pub wigle: WigleClient,
    pub apple: AppleClient,
    pub nominatim: NominatimClient,
    /// One-worker, latest-only reverse-geocode scheduler (task-0193). Locate
    /// handlers publish into its single pending slot; they never spawn a task.
    pub(crate) geocode: geocode::GeocodeScheduler,
    pub last_fix: tokio::sync::Mutex<Option<LastFix>>,
    /// BSSIDs currently undergoing remote provider lookup. Used by
    /// `resolver::resolve_chain` to coalesce concurrent requests for the
    /// same BSSID (scan loop + locate cold-start + pending drain can all
    /// fire at once otherwise). std::sync::Mutex is fine: we only ever hold
    /// it for an insert or a remove, never across await points, and that
    /// makes Drop-based cleanup viable for future RAII guards.
    pub inflight:
        std::sync::Mutex<std::collections::HashMap<String, crate::privacy::PrivacyTicket>>,
    /// Reverse-geocoded street addresses keyed by rounded (lat, lon).
    /// Populated lazily in the background after a `locate` returns; the
    /// next call at the same rounded position gets the address for free
    /// without blocking on Nominatim's 1 req/s rate limit.
    pub address_cache: std::sync::Mutex<AddressCache>,
    /// Cumulative count of best-effort DB writes that failed (warn-only
    /// sites). Surfaced via the stats endpoint (task-0076) so silent
    /// corruption is visible without scraping the journal.
    pub db_write_failures: Arc<std::sync::atomic::AtomicU64>,
    /// Executed WiGLE reservation attempts denied since daemon start because
    /// the same-day persisted API counter was malformed. Reset on restart.
    pub api_counter_corruption_denials: Arc<std::sync::atomic::AtomicU64>,
    /// Shared admission cap for unix and TCP connections. A permit is acquired
    /// before accept, so no Tokio task is spawned merely to wait for capacity.
    pub connection_slots: Arc<tokio::sync::Semaphore>,
    /// Single-flight admission for the O(N) history query/segmentation path.
    pub history_slot: Arc<tokio::sync::Semaphore>,
    /// Single-flight admission for destructive wipes. The accepted permit is
    /// shared by the handler response, worker, and durable DB continuation; an
    /// overlapping wipe cannot start until every surviving owner has finished.
    pub(crate) wipe_slot: Arc<tokio::sync::Semaphore>,
    /// Level-triggered cooperative shutdown signal (task-0190).
    ///
    /// Replaces the previous `tokio::sync::Notify`, whose `notify_waiters()`
    /// stored no permit: a SIGTERM landing while a background loop was
    /// mid-iteration (not parked in its `select!`) was lost, so the loop
    /// re-parked and ran forever. A `CancellationToken` is *sticky*: once
    /// `cancel()` is called, `cancelled()` is always-ready and
    /// `is_cancelled()` returns true, so no wakeup can be missed. Loops select
    /// their work/sleep against `cancelled()` and refuse to start new work
    /// once cancelled.
    pub shutdown: tokio_util::sync::CancellationToken,
    /// Background work and accepted connection handlers are tracked here so
    /// the shutdown drain can wait for every `DbHandle` user under one budget
    /// and force-abort any stragglers (task-0102/task-0192). A
    /// `std::sync::Mutex` guards
    /// the `JoinSet` because it is not `Sync` for concurrent spawn; the guard
    /// is only ever held to spawn/reap/take, never across an await.
    pub tasks: std::sync::Mutex<tokio::task::JoinSet<()>>,
    /// Accepted wipes are isolated from connection handlers in this
    /// single-flight task set. Main gives the set a bounded drain; the durable
    /// DB actor command owns post-commit memory clear + epoch invalidation, so
    /// aborting an async waiter cannot split those phases.
    pub(crate) privacy_wipes: std::sync::Mutex<tokio::task::JoinSet<()>>,
    /// Count of tracked tasks force-aborted at shutdown because they did not
    /// finish within the drain budget (task-0102). Surfaced via stats.
    pub orphan_tasks_aborted: std::sync::atomic::AtomicU64,
}

/// A cached last-known position with timestamp.
///
/// `at` is `chrono::DateTime<Utc>` rather than `std::time::Instant` so that
/// the value can be persisted to SQLite (`last_fix` table) and rehydrated
/// across daemon restarts. Wall-clock age is what users want anyway.
pub struct LastFix {
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: f64,
    pub address: Option<String>,
    pub at: chrono::DateTime<chrono::Utc>,
    pub sources: usize,
}

// task-0059: bridge in-memory LastFix and on-disk LastFixRow. The
// open-coded marshalling previously lived at three call sites in
// handle_locate, the address-backfill task, and main.rs::main.
impl From<&LastFix> for crate::db::LastFixRow {
    fn from(fix: &LastFix) -> Self {
        crate::db::LastFixRow {
            lat: fix.lat,
            lon: fix.lon,
            accuracy_m: fix.accuracy_m,
            address: fix.address.clone(),
            at_rfc3339: fix.at.to_rfc3339(),
            sources: fix.sources as i64,
        }
    }
}

impl TryFrom<crate::db::LastFixRow> for LastFix {
    type Error = anyhow::Error;
    fn try_from(row: crate::db::LastFixRow) -> Result<Self, Self::Error> {
        // task-0238: SQLite REAL columns can contain NaN/+-Inf. Reject the
        // persisted row at the rehydration choke point so main.rs discards it
        // (and warns with this root cause) instead of making a poisoned stale
        // fix available to return_last_fix_or_error.
        if !row.lat.is_finite() || !row.lon.is_finite() {
            anyhow::bail!(
                "non-finite last_fix coordinates: lat={}, lon={}",
                row.lat,
                row.lon
            );
        }
        let at = chrono::DateTime::parse_from_rfc3339(&row.at_rfc3339)
            .map_err(|e| {
                anyhow::anyhow!("unparseable last_fix timestamp {:?}: {e}", row.at_rfc3339)
            })?
            .with_timezone(&chrono::Utc);
        Ok(LastFix {
            lat: row.lat,
            lon: row.lon,
            accuracy_m: row.accuracy_m,
            address: row.address,
            at,
            sources: row.sources.max(0) as usize,
        })
    }
}

/// Update `last_fix.address` for a still-current position and persist the
/// resulting row while the in-memory `last_fix` guard remains held.
///
/// `persist` is the real persistence boundary rather than a test-only hook:
/// production passes the SQLite write, while the task-0046 regression wraps
/// that same write with an exact pre-DB signal. The explicit `drop(last)`
/// after `persist` is load-bearing — moving persistence below it recreates the
/// stale-backfill race where a newer in-memory fix can be overwritten on disk.
async fn backfill_last_fix_address_with<P, F>(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
    lat: f64,
    lon: f64,
    display: &str,
    persist: P,
) -> Result<bool>
where
    P: FnOnce(DbHandle, crate::db::LastFixRow) -> F + Send,
    F: std::future::Future<Output = Result<()>> + Send,
{
    let Some(_privacy) = state.privacy.read(ticket).await else {
        return Ok(false);
    };
    let mut last = state.last_fix.lock().await;
    let Some(fix) = last.as_mut() else {
        return Ok(false);
    };

    // Fail closed after task-0106 made address_cache_key return Option. Both
    // keys must be finite and equal; a poisoned persisted fix can never pick
    // up an address for an unrelated position.
    let keys_match = address_cache_key(fix.lat, fix.lon)
        .zip(address_cache_key(lat, lon))
        .is_some_and(|(a, b)| a == b);
    if !keys_match || fix.address.is_some() {
        return Ok(false);
    }

    fix.address = Some(display.to_owned());
    let row: crate::db::LastFixRow = (&*fix).into();
    let result = persist(state.db.clone(), row).await;
    // task-0046: prove the guard spans the persistence call. Do not move this
    // drop above `persist`; the concurrency regression asserts this ordering.
    drop(last);
    result?;
    Ok(true)
}

/// Persist a newly computed fix to both the in-memory and SQLite last-fix
/// stores (plus history) under the same `last_fix -> durable writer` ordering
/// used by address backfill. This is the handle_locate-side operation raced by the
/// task-0046 regression.
async fn persist_last_fix_and_history(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
    new_fix: LastFix,
) {
    let Some(_privacy) = state.privacy.read(ticket).await else {
        return;
    };
    let row: crate::db::LastFixRow = (&new_fix).into();
    let mut last = state.last_fix.lock().await;
    *last = Some(new_fix);

    if let Err(e) = state.db.persist_fix_and_history(row).await {
        warn!("failed to persist last_fix and history: {e}");
        state.record_db_failure();
    }
    drop(last);
}

/// Persist a newly computed locate and atomically publish its address-cache
/// observation to the geocode scheduler.
///
/// The `last_fix -> address_cache -> scheduler` critical section is
/// load-bearing. A newer locate cannot supersede generation A until its own
/// last_fix B is installed, so an A worker that races for backfill either
/// completes before supersession or sees B's mismatching rounded key. The
/// cache guard also makes miss publication atomic with worker insertion, then
/// is released before the potentially contended SQLite section. `last_fix`
/// remains held through DB persistence to preserve task-0046 ordering.
async fn persist_locate_fix_and_schedule(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
    mut new_fix: LastFix,
) -> Option<String> {
    let _privacy = state.privacy.read(ticket).await?;
    let mut last = state.last_fix.lock().await;
    let (address, row) = {
        let cache = state.address_cache.lock().unwrap_or_else(|e| {
            warn!("address cache mutex poisoned — recovering");
            e.into_inner()
        });
        let address = cache.get(new_fix.lat, new_fix.lon);
        new_fix.address = address.clone();
        let row: crate::db::LastFixRow = (&new_fix).into();
        *last = Some(new_fix);
        state
            .geocode
            .observe(row.lat, row.lon, address.is_none(), ticket);
        (address, row)
    };

    if let Err(e) = state.db.persist_fix_and_history(row).await {
        warn!("failed to persist last_fix and history: {e}");
        state.record_db_failure();
    }
    drop(last);
    address
}

#[cfg(test)]
mod last_fix_rehydration_tests {
    use super::*;

    fn row(lat: f64, lon: f64) -> crate::db::LastFixRow {
        crate::db::LastFixRow {
            lat,
            lon,
            accuracy_m: 25.0,
            address: Some("Copenhagen".to_string()),
            at_rfc3339: "2026-07-04T12:00:00+00:00".to_string(),
            sources: 3,
        }
    }

    /// task-0238 AC#1/#3: main.rs maps this conversion's Err to None. Cover
    /// every non-finite class in either coordinate; a finite counterpart
    /// remains present so this cannot regress into rejecting all persisted
    /// fixes.
    #[test]
    fn last_fix_rehydration_discards_non_finite_coordinates() {
        let poisoned = [
            (f64::NAN, 12.5, "NaN latitude"),
            (f64::INFINITY, 12.5, "+Inf latitude"),
            (f64::NEG_INFINITY, 12.5, "-Inf latitude"),
            (55.6, f64::NAN, "NaN longitude"),
            (55.6, f64::INFINITY, "+Inf longitude"),
            (55.6, f64::NEG_INFINITY, "-Inf longitude"),
        ];

        for (lat, lon, case) in poisoned {
            let persisted = row(lat, lon);
            let error = LastFix::try_from(persisted.clone())
                .err()
                .unwrap_or_else(|| panic!("{case} must return Err"));
            assert!(
                error
                    .to_string()
                    .contains("non-finite last_fix coordinates"),
                "{case} error must name the root cause, got {error}"
            );
            assert!(
                LastFix::try_from(persisted).ok().is_none(),
                "{case} must rehydrate as None at the startup call site"
            );
        }

        let finite = LastFix::try_from(row(55.6, 12.5)).expect("finite row must rehydrate");
        assert_eq!(finite.lat, 55.6);
        assert_eq!(finite.lon, 12.5);
    }
}

// --- Protocol types ---

/// Wire-protocol request, dispatched by the `cmd` field. task-0057
/// replaced a flat struct with optional fields; the new shape makes
/// illegal combinations (e.g. `cmd: locate` with `bssids` set) silently
/// ignored by the command's variant rather than carried as ghost
/// fields.
#[derive(Deserialize, Debug)]
#[serde(tag = "cmd", rename_all = "lowercase")]
enum Request {
    Locate,
    Resolve {
        #[serde(default)]
        bssids: Vec<String>,
    },
    Scan,
    Stats,
    Debug,
    Version,
    History {
        /// Relative range (e.g. "7d", "24h"). Mutually exclusive with from/to.
        #[serde(default)]
        range: Option<String>,
        /// Absolute start (RFC3339).
        #[serde(default)]
        from: Option<String>,
        /// Absolute end (RFC3339).
        #[serde(default)]
        to: Option<String>,
    },
    Wipe {
        #[serde(default)]
        confirm: Option<String>,
    },
}

#[derive(Serialize)]
struct ErrorResponse {
    ok: bool,
    v: u32,
    error: String,
}

#[derive(Serialize)]
struct LocateResponse {
    ok: bool,
    v: u32,
    lat: f64,
    lon: f64,
    accuracy_m: f64,
    sources: usize,
    cached: usize,
    fetched: usize,
    pending: usize,
    visible: usize,
    stable: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<String>,
    /// True if this is a stale last-known position (no current fix)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stale: bool,
    /// Age of the last-known position in seconds (only set when stale)
    #[serde(skip_serializing_if = "Option::is_none")]
    age_s: Option<u64>,
}

#[derive(Serialize)]
struct ResolveResponse {
    ok: bool,
    v: u32,
    results: Vec<ResolveResult>,
}

/// Provenance of a single `resolve` result (task-0062).
///
/// Distinct from the data-`Source` enum (apple/wigle/beacondb/manual/
/// unknown) which records WHO supplied the position. Provenance records
/// HOW the daemon obtained it for *this request*: from the on-disk cache,
/// fetched live during the request, or absent. The wire field name is
/// kept as `source` for backward compatibility with CLIs and curl scripts
/// that already parse the prior stringly-typed protocol.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Provenance {
    /// Fetched via a remote provider during this resolve call.
    Api,
    /// Served from the local SQLite cache.
    Cache,
    /// Authoritative absence: a provider returned NotFound for this BSSID on
    /// this call, or the not_found table recorded absence within its TTL.
    /// Safe for a client to cache as "not locatable".
    NotFound,
    /// task-0194: the daemon could not consult a provider for this BSSID this
    /// call, so absence is NOT established — the lookup was deferred to an
    /// in-flight resolution, the only provider was skipped (missing creds /
    /// exhausted or corrupt quota), or a transient failure occurred. Distinct
    /// from NotFound so clients that cache not_found don't blacklist a BSSID
    /// that is still resolvable; the client should retry later.
    Unresolved,
}

#[derive(Serialize)]
struct ResolveResult {
    bssid: String,
    lat: Option<f64>,
    lon: Option<f64>,
    ssid: Option<String>,
    /// Wire field name stays `source` for backward compat. Internally
    /// `provenance` to disambiguate from the data-source `Source` enum
    /// (task-0062).
    #[serde(rename = "source")]
    provenance: Provenance,
}

#[derive(Serialize)]
struct ScanResponse {
    ok: bool,
    v: u32,
    networks: Vec<NetworkInfo>,
    scan_age_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scanned_at: Option<String>,
}

#[derive(Serialize)]
struct NetworkInfo {
    bssid: String,
    ssid: Option<String>,
    signal_dbm: i32,
    channel: Option<i32>,
}

#[derive(Serialize)]
struct StatsResponse {
    ok: bool,
    v: u32,
    cached_aps: i64,
    pending_aps: i64,
    not_found_aps: i64,
    db_size_bytes: i64,
    api_calls_today: u32,
    /// Executed reservation attempts denied since daemon start because the
    /// same-day persisted API counter was malformed. Reset on restart.
    api_counter_corruption_denials: u64,
    /// Number of BSSIDs currently undergoing remote provider lookup
    /// (size of state.inflight). task-0074.
    inflight: usize,
    /// Cumulative count of best-effort DB writes that failed and were
    /// warned-only. Helps spot silent corruption (full disk, missing
    /// write permissions, etc.). Reset on daemon restart. task-0076.
    db_write_failures: u64,
    /// Number of background tasks (tracked resolve/backfill spawns and the
    /// named scan/pending/geocode/prune/server loops) force-aborted at the end
    /// of the LAST clean shutdown because they did not finish within the
    /// drain budget. Persisted at shutdown and reloaded at startup — the count is
    /// produced after the server socket closes, so the previous run's value
    /// is the only observable one. A crash/SIGKILL does not update it.
    /// task-0102.
    orphan_tasks_aborted: u64,
}

#[derive(Serialize)]
struct WipeResponse {
    ok: bool,
    v: u32,
    aps_deleted: usize,
    pending_deleted: usize,
    not_found_deleted: usize,
    fixes_deleted: usize,
    last_fix_deleted: usize,
}

fn error_json(msg: &str) -> String {
    serde_json::to_string(&ErrorResponse {
        ok: false,
        v: 1,
        error: msg.to_string(),
    })
    .unwrap_or_else(|_| r#"{"ok":false,"v":1,"error":"serialization failed"}"#.to_string())
}

/// Serialize a response value or fall back to a generic error_json.
/// Replaces the eight identical `serde_json::to_string(&resp).unwrap_or_else(...)`
/// tails (task-0058).
fn ok_json<T: Serialize>(t: &T) -> String {
    serde_json::to_string(t).unwrap_or_else(|_| error_json("serialization failed"))
}

/// A wire response plus any resource whose lifetime must extend through the
/// socket write and shutdown. History uses this to keep its single-flight slot
/// occupied until the response write terminates by success, failure, timeout,
/// disconnect, or handler cancellation.
struct CommandResponse {
    body: String,
    _history_lease: Option<Arc<tokio::sync::OwnedSemaphorePermit>>,
    _wipe_admission: Option<Arc<WipeAdmission>>,
    privacy: ResponsePrivacy,
}

struct WipeAdmission {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

/// Actor-owned invariants that must survive unwinding until process fail-stop
/// has begun. Keeping both in the guarded call prevents privacy publication or
/// a second wipe from being admitted during fatal handling.
struct DurableWipeGuard {
    lease: crate::privacy::PrivacyWipeLease,
    _admission: Arc<WipeAdmission>,
    #[cfg(test)]
    _drop_marker: WipeGuardDropMarker,
}

impl DurableWipeGuard {
    fn new(lease: crate::privacy::PrivacyWipeLease, admission: Arc<WipeAdmission>) -> Self {
        Self {
            lease,
            _admission: admission,
            #[cfg(test)]
            _drop_marker: WipeGuardDropMarker(
                std::env::var_os(WIPE_GUARD_DROP_MARKER_ENV).map(std::path::PathBuf::from),
            ),
        }
    }
}

#[cfg(test)]
struct WipeGuardDropMarker(Option<std::path::PathBuf>);

#[cfg(test)]
impl Drop for WipeGuardDropMarker {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::write(path, b"dropped");
        }
    }
}

enum ResponsePrivacy {
    None,
    Pending(crate::privacy::PrivacyTicket),
    Read {
        _lease: crate::privacy::PrivacyReadLease,
    },
    Wipe {
        _lease: crate::privacy::PrivacyWipeLease,
    },
}

impl CommandResponse {
    fn history(
        body: String,
        lease: Arc<tokio::sync::OwnedSemaphorePermit>,
        ticket: crate::privacy::PrivacyTicket,
    ) -> Self {
        Self {
            body,
            _history_lease: Some(lease),
            _wipe_admission: None,
            privacy: ResponsePrivacy::Pending(ticket),
        }
    }

    fn privacy(body: String, ticket: crate::privacy::PrivacyTicket) -> Self {
        Self {
            body,
            _history_lease: None,
            _wipe_admission: None,
            privacy: ResponsePrivacy::Pending(ticket),
        }
    }

    fn wipe(
        body: String,
        lease: crate::privacy::PrivacyWipeLease,
        admission: Arc<WipeAdmission>,
    ) -> Self {
        Self {
            body,
            _history_lease: None,
            _wipe_admission: Some(admission),
            privacy: ResponsePrivacy::Wipe { _lease: lease },
        }
    }

    fn wipe_error(body: String, admission: Arc<WipeAdmission>) -> Self {
        Self {
            body,
            _history_lease: None,
            _wipe_admission: Some(admission),
            privacy: ResponsePrivacy::None,
        }
    }

    /// Acquire the response-side privacy boundary immediately before socket
    /// output. A wipe that won while a handler was computing replaces the stale
    /// body with a stable retry error. The resulting read/write lease remains in
    /// this response until write+shutdown completes or times out.
    async fn prepare_for_write(&mut self, state: &DaemonState) {
        let pending = match std::mem::replace(&mut self.privacy, ResponsePrivacy::None) {
            ResponsePrivacy::Pending(ticket) => Some(ticket),
            held => {
                self.privacy = held;
                None
            }
        };
        let Some(ticket) = pending else {
            return;
        };
        match state.privacy.read(ticket).await {
            Some(lease) => self.privacy = ResponsePrivacy::Read { _lease: lease },
            None => self.body = error_json(STALE_AFTER_WIPE_ERROR),
        }
    }
}

impl From<String> for CommandResponse {
    fn from(body: String) -> Self {
        Self {
            body,
            _history_lease: None,
            _wipe_admission: None,
            privacy: ResponsePrivacy::None,
        }
    }
}

impl std::ops::Deref for CommandResponse {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.body
    }
}

impl std::fmt::Display for CommandResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.body)
    }
}

// --- Unix socket transport setup (task-0196) ---

/// systemd socket-activation fd number. `sd_listen_fds(3)` hands the first
/// passed socket as fd 3 (`SD_LISTEN_FDS_START`).
pub const SD_LISTEN_FDS_START: i32 = 3;

/// Minimal `sd_listen_fds` parse (avoids a libsystemd dependency). Returns the
/// activated fd when systemd passed us exactly one socket addressed to THIS
/// process, else `None` (task-0196).
///
/// - `LISTEN_PID` must equal our pid: systemd sets it to the service PID so a
///   forked child does not accidentally inherit the listening fds.
/// - `LISTEN_FDS` must be exactly 1; whereamid uses a single control socket, so
///   more than one is a misconfiguration and a hard startup error.
pub fn sd_listen_fd() -> Result<Option<i32>> {
    parse_listen_fds(
        std::env::var("LISTEN_PID").ok().as_deref(),
        std::env::var("LISTEN_FDS").ok().as_deref(),
        std::process::id(),
    )
}

/// Pure core of [`sd_listen_fd`] (env reads split out so the parse logic is
/// unit-testable without mutating the process environment). task-0196.
fn parse_listen_fds(
    listen_pid: Option<&str>,
    listen_fds: Option<&str>,
    my_pid: u32,
) -> Result<Option<i32>> {
    let (pid, fds) = match (listen_pid, listen_fds) {
        (Some(p), Some(f)) => (p, f),
        _ => return Ok(None), // not socket-activated
    };
    let pid: u32 = pid
        .parse()
        .with_context(|| format!("LISTEN_PID={pid:?} is not an integer"))?;
    if pid != my_pid {
        // The fds were meant for a different process (we inherited the env);
        // ignore socket activation and fall through to self-bind.
        return Ok(None);
    }
    let fds: i32 = fds
        .parse()
        .with_context(|| format!("LISTEN_FDS={fds:?} is not an integer"))?;
    if fds < 1 {
        return Ok(None);
    }
    if fds > 1 {
        anyhow::bail!("LISTEN_FDS={fds}: whereamid accepts exactly one socket-activated fd");
    }
    Ok(Some(SD_LISTEN_FDS_START))
}

/// Verify an adopted fd really is a *listening AF_UNIX* socket (what
/// `sd_is_socket_unix` does in libsystemd). `parse_listen_fds` only validates
/// the LISTEN_* env integers; a wrong fd from a broken parent would otherwise
/// surface later as a confusing accept/epoll error instead of a clean startup
/// failure (task-0196 review hardening, mped M3).
///
/// - `local_addr()` (getsockname) fails with ENOTSOCK on a non-socket fd and
///   with an address-family error on a non-unix socket.
/// - `SO_ACCEPTCONN` distinguishes a listening socket from a connected one.
pub fn verify_unix_listener_fd(listener: &std::os::unix::net::UnixListener) -> Result<()> {
    use std::os::unix::io::AsRawFd;
    listener
        .local_addr()
        .context("socket-activated fd is not an AF_UNIX socket (getsockname failed)")?;
    let fd = listener.as_raw_fd();
    let mut accepting: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // Safety: fd is owned by `listener` (open for its lifetime); `accepting`
    // and `len` are valid, correctly-sized out-parameters for this sockopt.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ACCEPTCONN,
            &mut accepting as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error())
            .context("getsockopt(SO_ACCEPTCONN) on socket-activated fd");
    }
    if accepting == 0 {
        anyhow::bail!(
            "socket-activated fd is an AF_UNIX socket but is not listening (SO_ACCEPTCONN=0); \
             expected a ListenStream= socket from systemd"
        );
    }
    Ok(())
}

/// Set FD_CLOEXEC on an adopted fd. systemd passes activation fds WITHOUT
/// close-on-exec (by design — the service is meant to inherit them), but this
/// daemon spawns `nmcli`/`iw` children that must not inherit the control
/// socket. Self-bound sockets are unaffected (Rust binds with SOCK_CLOEXEC);
/// only the socket-activation path needs this (task-0196 review hardening,
/// mped L1).
pub fn set_cloexec(fd: i32) -> Result<()> {
    // Safety: plain fcntl on an fd the caller owns; no memory is passed.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error()).context("fcntl(F_GETFD)");
    }
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error()).context("fcntl(F_SETFD, FD_CLOEXEC)");
    }
    Ok(())
}

/// Bind a Unix domain socket at `path`, recovering from a stale socket file and
/// tightening the result to owner-only (0600). Fails closed (task-0196):
///
/// - Path exists and a live daemon answers a connect probe → REFUSE (never
///   steal a live socket).
/// - Path exists but connect is refused → stale inode from a daemon that died
///   without cleanup; unlink and bind.
/// - Path exists but is not a socket → REFUSE (never clobber an unrelated file).
/// - After bind, chmod 0600 and RE-STAT; a filesystem that silently ignored the
///   chmod (FUSE, some network mounts) would leave the control channel
///   world-accessible, so bail rather than serve it loose (mirrors the
///   fail-closed DB chmod in db.rs, commit 53aba50).
///
/// Returns a std listener; the caller sets non-blocking and adopts it into
/// tokio. Kept sync + std so it is unit-testable without a runtime.
pub fn self_bind_unix(path: &Path) -> Result<std::os::unix::net::UnixListener> {
    handle_stale_socket(path)?;
    // Bind under a tightened umask so the socket is never observable looser
    // than 0600, even for an instant. Without this, a caller umask of e.g.
    // 0022 leaves a group/other-writable socket in the window before the
    // chmod below (task-0196 review hardening, mped L2). Tightening is the
    // safe direction for any file another thread might create concurrently.
    // Safety: umask() only swaps an integer in process state; no memory.
    let old_umask = unsafe { libc::umask(0o077) };
    let bind_result = std::os::unix::net::UnixListener::bind(path);
    unsafe { libc::umask(old_umask) };
    let listener =
        bind_result.with_context(|| format!("binding unix socket {}", path.display()))?;
    // Belt and braces: the umask handles the window; the chmod + fail-closed
    // re-stat guarantees the FINAL mode even on filesystems that ignore mode
    // bits at bind time.
    chmod_socket_0600_or_bail(path)?;
    Ok(listener)
}

/// Probe an existing socket path and either clear a stale inode or refuse.
fn handle_stale_socket(path: &Path) -> Result<()> {
    use std::os::unix::fs::FileTypeExt;
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("stat {}", path.display())),
    };
    // Only ever unlink a socket. A regular file / symlink at this path is not
    // ours to remove — fail loudly instead of clobbering it.
    if !meta.file_type().is_socket() {
        anyhow::bail!(
            "{} exists and is not a socket; refusing to remove it. Choose a different --socket \
             path or remove the file manually.",
            path.display()
        );
    }
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => anyhow::bail!(
            "another process is already listening on {} (connect probe succeeded); refusing to \
             steal a live control socket. Stop the running whereamid or pass a different --socket.",
            path.display()
        ),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            // Stale socket inode from a daemon that exited without cleanup.
            info!("removing stale socket file {}", path.display());
            match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                // Vanished between stat and unlink (race): fine, it is gone.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => {
                    Err(e).with_context(|| format!("removing stale socket {}", path.display()))
                }
            }
        }
        Err(e) => Err(e)
            .with_context(|| format!("probing existing socket {} before bind", path.display())),
    }
}

/// chmod a freshly-bound socket to 0600 and verify the mode actually took.
fn chmod_socket_0600_or_bail(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 0600 on socket {}", path.display()))?;
    let mode = std::fs::metadata(path)
        .with_context(|| format!("re-stat socket {} after chmod", path.display()))?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        anyhow::bail!(
            "socket {} is still group/other-accessible (mode {mode:o}) after chmod 0600 — the \
             filesystem ignored the permission change; the control channel would be exposed to \
             other local users. Refusing to start.",
            path.display()
        );
    }
    Ok(())
}

/// Run the control-channel accept loop until cancelled. The unix listener (the
/// mandatory transport) is bound by the caller — a bind failure there aborts
/// startup with a non-zero exit. TCP is opt-in and bound HERE; if its bind
/// fails the daemon degrades to unix-only and warns loudly rather than exiting,
/// so a busy opt-in port never takes down the mandatory transport (task-0233,
/// amending the task-0196 behaviour where a TCP-bind failure self-exited).
pub async fn run_server(state: Arc<DaemonState>, unix: UnixListener) -> Result<()> {
    let tcp: Option<TcpListener> = match state.args.parsed_bind()? {
        Some(addr) => match TcpListener::bind(addr).await {
            Ok(listener) => {
                let bound = listener.local_addr().unwrap_or(addr);
                if bound.ip().is_loopback() {
                    warn!(
                        "TCP listening on {bound}: this transport has NO peer authentication — \
                         every local user can read location history, live position and Wi-Fi \
                         scans and drive WiGLE quota. The unix socket is the access-controlled \
                         transport."
                    );
                } else {
                    warn!(
                        "TCP listening on {bound} (non-loopback): location history, live position, \
                         Wi-Fi scans and WiGLE quota are reachable by every host that can reach \
                         {bound}, with NO authentication."
                    );
                }
                Some(listener)
            }
            // task-0233: the opt-in TCP transport failed to bind (e.g.
            // EADDRINUSE from a squatted port). Do NOT abort the daemon —
            // degrade to unix-only and warn loudly. The unix socket is the
            // mandatory, access-controlled transport and is already bound; the
            // operator asked for TCP as an ADDITION, so its failure must not
            // take the whole daemon down. Making it fatal would also crash-loop
            // the daemon to start-limit-hit under systemd Restart=on-failure
            // (task-0204) whenever the opt-in port is momentarily busy.
            Err(e) => {
                warn!(
                    "requested --bind {addr} failed to bind ({e}); continuing on the unix \
                     control socket ONLY. The opt-in TCP transport is UNAVAILABLE this run."
                );
                None
            }
        },
        None => None,
    };
    info!("control channel: unix socket ready");

    run_server_inner(
        Arc::clone(&state),
        unix,
        tcp,
        Arc::clone(&state.connection_slots),
    )
    .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransportKind {
    Unix,
    Tcp,
}

/// Accept-loop core with injectable admission state for deterministic tests.
/// A permit is acquired before polling either listener. Consequently excess
/// connections remain in the kernel backlog and never become permit-waiting
/// Tokio tasks. Unix and TCP draw from the same semaphore.
async fn run_server_inner(
    state: Arc<DaemonState>,
    unix: UnixListener,
    tcp: Option<TcpListener>,
    connection_slots: Arc<tokio::sync::Semaphore>,
) -> Result<()> {
    enum Accepted {
        Unix(std::io::Result<(tokio::net::UnixStream, tokio::net::unix::SocketAddr)>),
        Tcp(std::io::Result<(tokio::net::TcpStream, std::net::SocketAddr)>),
    }

    loop {
        let permit = Arc::clone(&connection_slots)
            .acquire_owned()
            .await
            .context("connection admission semaphore closed")?;
        let accepted = tokio::select! {
            r = unix.accept() => Accepted::Unix(r),
            r = async {
                match &tcp {
                    Some(l) => l.accept().await,
                    // No TCP configured: never resolve, so this branch is inert
                    // and the unix branch drives the loop (task-0196).
                    None => std::future::pending().await,
                }
            } => Accepted::Tcp(r),
        };

        match accepted {
            Accepted::Unix(r) => match r {
                Ok((stream, _addr)) => {
                    // SO_PEERCRED is logged for auditability only. We do NOT
                    // enforce same-UID in-process: the system-service model has
                    // systemd hand a group-readable socket (0660 root:whereami)
                    // to a DynamicUser daemon, so the connecting UID legitimately
                    // differs from the daemon UID. Filesystem permissions on the
                    // socket ARE the access control (ADR-003).
                    match stream.peer_cred() {
                        Ok(cred) => debug!(
                            "unix connection uid={} gid={} pid={:?}",
                            cred.uid(),
                            cred.gid(),
                            cred.pid()
                        ),
                        Err(e) => debug!("unix connection (peer_cred unavailable: {e})"),
                    }
                    spawn_handler(stream, Arc::clone(&state), permit, TransportKind::Unix);
                }
                Err(e) => error!("unix accept error: {e}"),
            },
            Accepted::Tcp(r) => match r {
                Ok((stream, addr)) => {
                    debug!("tcp connection from {addr}");
                    spawn_handler(stream, Arc::clone(&state), permit, TransportKind::Tcp);
                }
                Err(e) => error!("tcp accept error: {e}"),
            },
        }
    }
}

/// Spawn a per-connection handler generic over the stream type. Tracking is
/// required by the database actor lifecycle: shutdown must drain every holder
/// of `DbHandle` before it sends the actor stop commands. Admission is already
/// owned by the supplied permit, shared across both transports (task-0201).
fn spawn_handler<S>(
    stream: S,
    state: Arc<DaemonState>,
    connection_permit: tokio::sync::OwnedSemaphorePermit,
    transport: TransportKind,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let task_state = Arc::clone(&state);
    state.spawn_tracked(async move {
        // Own admission for the complete read/dispatch/write/shutdown lifetime.
        // Refusal or abort drops the future and therefore returns capacity.
        let _connection_permit = connection_permit;
        if let Err(e) = handle_connection(stream, task_state, transport).await {
            warn!("connection error: {e}");
        }
    });
}

/// Handle one request/response exchange. Generic over the transport so the unix
/// socket and opt-in TCP share identical framing, limits and timeouts
/// (task-0196). `tokio::io::split` works for any `AsyncRead + AsyncWrite`,
/// unlike the type-specific `into_split`.
async fn handle_connection<S>(
    stream: S,
    state: Arc<DaemonState>,
    transport: TransportKind,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let limited = reader.take(MAX_REQUEST_BYTES);
    let mut buf_reader = BufReader::new(limited);
    let mut line = String::new();

    // Read one line with timeout
    let n = match timeout(READ_TIMEOUT, buf_reader.read_line(&mut line)).await {
        Ok(Ok(n)) => n,
        Ok(Err(e)) => {
            writer
                .write_all(format!("{}\n", error_json(&format!("read error: {e}"))).as_bytes())
                .await
                .ok();
            return Ok(());
        }
        Err(_) => {
            writer
                .write_all(format!("{}\n", error_json("read timeout")).as_bytes())
                .await
                .ok();
            return Ok(());
        }
    };
    if n == 0 {
        return Ok(());
    }

    let mut response = match serde_json::from_str::<Request>(line.trim()) {
        Ok(req) => dispatch_command_for_transport(&req, &state, transport).await,
        Err(e) => error_json(&format!("invalid request: {e}")).into(),
    };
    response.prepare_for_write(&state).await;

    // Bound the write half of the connection. A slow-reading client
    // could otherwise keep the response buffered and pin a tokio task
    // forever (task-0080).
    let write_response = async {
        writer.write_all(response.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.shutdown().await
    };
    if timeout(WRITE_TIMEOUT, write_response).await.is_err() {
        debug!("write timeout closing connection");
        return Ok(());
    }

    Ok(())
}

#[cfg(test)]
async fn dispatch_command(req: &Request, state: &Arc<DaemonState>) -> CommandResponse {
    dispatch_command_for_transport(req, state, TransportKind::Unix).await
}

async fn dispatch_command_for_transport(
    req: &Request,
    state: &Arc<DaemonState>,
    transport: TransportKind,
) -> CommandResponse {
    let ticket = state.privacy.ticket();
    match req {
        Request::Locate => CommandResponse::privacy(handle_locate(state, ticket).await, ticket),
        Request::Resolve { bssids } => {
            CommandResponse::privacy(handle_resolve(bssids, state, ticket).await, ticket)
        }
        Request::Scan => CommandResponse::privacy(handle_scan(state).await, ticket),
        Request::Stats => CommandResponse::privacy(handle_stats(state).await, ticket),
        Request::Debug => CommandResponse::privacy(handle_debug(state).await, ticket),
        Request::Version => handle_version().into(),
        Request::History { range, from, to } => {
            handle_history(
                range.as_deref(),
                from.as_deref(),
                to.as_deref(),
                state,
                ticket,
            )
            .await
        }
        Request::Wipe { confirm } => handle_wipe(confirm.as_deref(), state, transport).await,
    }
}

async fn handle_locate(state: &Arc<DaemonState>, ticket: crate::privacy::PrivacyTicket) -> String {
    use crate::resolver;
    use crate::trilaterate::{trilaterate, PositionedAp};

    let Some(privacy) = state.privacy.read(ticket).await else {
        return error_json(STALE_AFTER_WIPE_ERROR);
    };
    let debouncer = state.debouncer.lock().await;
    let stable = debouncer.stable_bssids();
    let visible = debouncer.latest_scan().map(|s| s.len()).unwrap_or(0);

    // Build candidate list: stable APs with a CURRENT signal, sorted by
    // RSSI, top-N. Stable BSSIDs that fell out of the most recent scan
    // (no current signal) are filtered out rather than fed a fake -90 dBm
    // (task-0051) — fake readings poison trilateration weights, which is
    // exactly the bug task-0043 fixed in the nmcli parser.
    let stable_count = stable.len();
    let mut candidates: Vec<(String, i32)> = stable
        .iter()
        .filter_map(|b| debouncer.latest_signal(b).map(|s| (b.clone(), s)))
        .collect();
    candidates.sort_by_key(|c| std::cmp::Reverse(c.1));
    candidates.truncate(state.args.top_n);

    // Cold-start fallback: if no stable APs have cached positions,
    // use ALL visible APs from latest scan (bypass debounce for the query).
    // They won't be committed to cache — just used for this one response.
    let fallback_candidates: Vec<(String, i32)> = if candidates.is_empty() {
        debouncer
            .latest_scan()
            .map(|scan| {
                let mut v: Vec<(String, i32)> = scan
                    .iter()
                    .map(|(b, e)| (b.clone(), e.signal_dbm))
                    .collect();
                v.sort_by_key(|c| std::cmp::Reverse(c.1));
                v.truncate(state.args.top_n);
                v
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    drop(debouncer);
    drop(privacy);

    // Try stable candidates first, fall back to raw visible APs
    let using_fallback = candidates.is_empty() && !fallback_candidates.is_empty();
    let active_candidates = if candidates.is_empty() {
        &fallback_candidates
    } else {
        &candidates
    };

    if active_candidates.is_empty() {
        return return_last_fix_or_error(
            state,
            ticket,
            "no APs detected yet",
            visible,
            stable_count,
        )
        .await;
    }

    let bssids: Vec<String> = active_candidates.iter().map(|(b, _)| b.clone()).collect();
    let signal_map: std::collections::HashMap<String, i32> =
        active_candidates.iter().cloned().collect();

    // Cache-only lookup — never blocks on WiGLE. Instant response.
    let cached_aps = resolver::lookup_cached(&bssids, state, ticket).await;
    let cached_count = cached_aps.len();

    if cached_aps.is_empty() {
        // Nothing cached. If using fallback, queue all for background resolution.
        if using_fallback {
            // task-0102: track so the shutdown drain waits for this resolve
            // instead of killing it mid-API-call (which would leak a WiGLE
            // quota lease / leave partial writes).
            let resolve_state = Arc::clone(state);
            let bssids_clone = bssids.clone();
            state.spawn_tracked(async move {
                resolver::resolve_background(&bssids_clone, &resolve_state, ticket).await;
            });
        }
        return return_last_fix_or_error(
            state,
            ticket,
            "no cached positions for visible APs (resolving in background)",
            visible,
            stable_count,
        )
        .await;
    }

    // Build trilateration input.
    //
    // task-0227: SQLite REAL columns can hold NaN/±Inf, and a non-finite
    // coordinate flows straight through to_unit_vec -> NaN vector -> the
    // `mag > 0` guards are false-on-NaN -> from_unit_vec returns (NaN, NaN)
    // as a "successful" Ok(Position). Skip such rows here (first line of
    // defense) and name the offending bssid so the poisoned cache row is
    // traceable. trilaterate() re-filters as defense in depth for callers
    // that bypass this path.
    let positioned_aps: Vec<PositionedAp> = cached_aps
        .iter()
        .filter(|ap| {
            let finite = ap.lat.is_finite() && ap.lon.is_finite();
            if !finite {
                warn!(
                    "skipping AP {}: non-finite cached position ({}, {})",
                    ap.bssid, ap.lat, ap.lon
                );
            }
            finite
        })
        .map(|ap| PositionedAp {
            lat: ap.lat,
            lon: ap.lon,
            signal_dbm: signal_map.get(&ap.bssid).copied(),
        })
        .collect();

    match trilaterate(&positioned_aps) {
        Ok(pos) => {
            // Save as last-known fix: in-memory + SQLite, both under the
            // last_fix mutex (task-0046).
            //
            // Holding the last_fix mutex across the DB write closes the race
            // where two concurrent handle_locate calls could otherwise have
            // their in-memory writes interleave with their DB writes:
            //   T1 in-mem X, T1 drops, T2 in-mem Y, T2 drops + persists Y,
            //   T1 persists X => disk diverges from in-memory.
            //
            // The last_fix guard intentionally spans the durable writer-actor
            // await. This serializes memory publication with DB FIFO ordering;
            // the address-backfill path uses the same order. last_fix and
            // history are committed together in one actor transaction.
            let new_fix = LastFix {
                lat: pos.lat,
                lon: pos.lon,
                accuracy_m: pos.accuracy_m,
                address: None,
                at: chrono::Utc::now(),
                sources: cached_count,
            };
            // Address resolution remains off the response-critical network
            // path. An address-enabled locate atomically persists its fix and
            // observes the rounded cache cell; misses replace one bounded
            // scheduler slot and hits supersede stale rate-waiting work.
            let address = if state.args.address_approx {
                persist_locate_fix_and_schedule(state, ticket, new_fix).await
            } else {
                persist_last_fix_and_history(state, ticket, new_fix).await;
                None
            };

            let resp = LocateResponse {
                ok: true,
                v: 1,
                lat: pos.lat,
                lon: pos.lon,
                accuracy_m: pos.accuracy_m,
                sources: cached_count,
                cached: cached_count,
                fetched: 0,
                pending: 0,
                visible,
                stable: stable_count,
                address,
                stale: false,
                age_s: None,
            };
            ok_json(&resp)
        }
        Err(e) => {
            return_last_fix_or_error(
                state,
                ticket,
                &format!("trilateration failed: {e}"),
                visible,
                stable_count,
            )
            .await
        }
    }
}

/// Return last-known position if available, otherwise return error.
async fn return_last_fix_or_error(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
    error_msg: &str,
    visible: usize,
    stable: usize,
) -> String {
    let Some(_privacy) = state.privacy.read(ticket).await else {
        return error_json(STALE_AFTER_WIPE_ERROR);
    };
    let last = state.last_fix.lock().await;
    if let Some(fix) = last.as_ref() {
        // Wall-clock age. Negative ages (clock skew) clamp to zero.
        let age_s = (chrono::Utc::now() - fix.at).num_seconds().max(0) as u64;
        let resp = LocateResponse {
            ok: true,
            v: 1,
            lat: fix.lat,
            lon: fix.lon,
            accuracy_m: fix.accuracy_m,
            sources: fix.sources,
            cached: 0,
            fetched: 0,
            pending: 0,
            visible,
            stable,
            address: fix.address.clone(),
            stale: true,
            age_s: Some(age_s),
        };
        ok_json(&resp)
    } else {
        error_json(error_msg)
    }
}

async fn handle_resolve(
    bssids: &[String],
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
) -> String {
    use crate::resolver;

    if bssids.is_empty() {
        return error_json("resolve requires non-empty bssids array");
    }
    if bssids.len() > MAX_RESOLVE_BSSIDS {
        return error_json(&format!(
            "resolve accepts at most {MAX_RESOLVE_BSSIDS} BSSIDs per request (got {})",
            bssids.len()
        ));
    }

    let normalized: Vec<String> = bssids
        .iter()
        .map(|b| crate::scanner::normalize_bssid(b))
        .collect();

    let result = resolver::resolve_readonly(&normalized, state, ticket).await;

    let results: Vec<ResolveResult> = normalized
        .iter()
        .map(|bssid| {
            match result.positioned.iter().find(|a| &a.bssid == bssid) {
                Some(ap) if ap.lat.is_finite() && ap.lon.is_finite() => ResolveResult {
                    bssid: bssid.clone(),
                    lat: Some(ap.lat),
                    lon: Some(ap.lon),
                    ssid: ap.ssid.clone(),
                    provenance: if result.fetched_bssids.contains(bssid) {
                        Provenance::Api
                    } else {
                        Provenance::Cache
                    },
                },
                // task-0238: fail closed at the protocol boundary. A poisoned
                // cache row is not authoritative absence, so classify it as
                // unresolved (retryable), never not_found. This guard also
                // protects against a future provider returning non-finite
                // coordinates directly in this read-only path.
                Some(ap) => {
                    warn!(
                        "discarding resolve position for {}: non-finite coordinates ({}, {})",
                        ap.bssid, ap.lat, ap.lon
                    );
                    ResolveResult {
                        bssid: bssid.clone(),
                        lat: None,
                        lon: None,
                        ssid: None,
                        provenance: Provenance::Unresolved,
                    }
                }
                None => {
                    // task-0194: only report not_found when the resolver
                    // established *authoritative* absence for this call (a provider
                    // said NotFound, or the not_found table did within TTL).
                    // Everything else — deferred to an in-flight resolution, a
                    // provider skipped for missing creds / exhausted or corrupt
                    // quota, or a transient failure — is `unresolved`: absence
                    // is NOT established, so a client must retry rather than blacklist it.
                    let provenance = if result.not_found.contains(bssid) {
                        Provenance::NotFound
                    } else {
                        Provenance::Unresolved
                    };
                    ResolveResult {
                        bssid: bssid.clone(),
                        lat: None,
                        lon: None,
                        ssid: None,
                        provenance,
                    }
                }
            }
        })
        .collect();

    let resp = ResolveResponse {
        ok: true,
        v: 1,
        results,
    };
    ok_json(&resp)
}

async fn handle_scan(state: &Arc<DaemonState>) -> String {
    let debouncer = state.debouncer.lock().await;
    let scan_age_ms = debouncer.latest_scan_age_ms();
    let scanned_at = debouncer.latest_scan_time().map(|t| t.to_rfc3339());
    let networks = match debouncer.latest_scan() {
        Some(sample) => {
            let mut nets: Vec<NetworkInfo> = sample
                .iter()
                .map(|(bssid, entry)| NetworkInfo {
                    bssid: bssid.clone(),
                    ssid: entry.ssid.clone(),
                    signal_dbm: entry.signal_dbm,
                    channel: entry.channel,
                })
                .collect();
            nets.sort_by_key(|n| std::cmp::Reverse(n.signal_dbm));
            nets
        }
        None => Vec::new(),
    };

    let resp = ScanResponse {
        ok: true,
        v: 1,
        networks,
        scan_age_ms,
        scanned_at,
    };
    ok_json(&resp)
}

/// DB classification of a BSSID seen by the debouncer (task-0062).
/// Wire format is the lowercase string the previous stringly-typed
/// implementation emitted; see `Provenance` for the rename rationale.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DbStatus {
    /// BSSID is in the `aps` cache.
    Cached,
    /// BSSID is queued for resolution.
    Pending,
    /// BSSID is in the `not_found` table (definitively absent at every
    /// provider, within the TTL window).
    NotFound,
    /// BSSID has not been observed by the daemon before this scan.
    New,
}

#[derive(Serialize)]
struct DebugBssid {
    bssid: String,
    /// None when the BSSID is debounce-stable but absent from the most
    /// recent scan (was previously fabricated as -90 dBm; task-0051).
    signal_dbm: Option<i32>,
    seen: usize,   // times seen in debounce window
    needed: usize, // threshold to be stable
    is_stable: bool,
    db_status: DbStatus,
}

#[derive(Serialize)]
struct DebugResponse {
    ok: bool,
    v: u32,
    daemon_rev: &'static str,
    scan_age_ms: Option<u64>,
    samples_in_buffer: usize,
    visible: usize,
    stable: usize,
    bssids: Vec<DebugBssid>,
}

async fn handle_debug(state: &Arc<DaemonState>) -> String {
    let debouncer = state.debouncer.lock().await;
    let scan_age_ms = debouncer.latest_scan_age_ms();
    let samples = debouncer.sample_count();
    let threshold = debouncer.threshold();
    let stable_set = debouncer.stable_bssids();

    // Collect ALL BSSIDs ever seen in the ring buffer. Signals from the
    // latest scan are Some(_); stable BSSIDs that fell out of the latest
    // scan are None (task-0051).
    let mut all_bssids: std::collections::HashMap<String, Option<i32>> =
        std::collections::HashMap::new();
    if let Some(scan) = debouncer.latest_scan() {
        for (bssid, entry) in scan {
            all_bssids.insert(bssid.clone(), Some(entry.signal_dbm));
        }
    }
    // Also include stable ones not in latest scan with signal=None.
    for b in &stable_set {
        all_bssids.entry(b.clone()).or_insert(None);
    }

    let visible = debouncer.latest_scan().map(|s| s.len()).unwrap_or(0);

    let debug_inputs: Vec<(String, Option<i32>, usize, bool)> = all_bssids
        .iter()
        .map(|(bssid, &signal)| {
            (
                bssid.clone(),
                signal,
                debouncer.count(bssid),
                stable_set.contains(bssid),
            )
        })
        .collect();
    drop(debouncer);

    let statuses = state
        .db
        .debug_statuses(
            debug_inputs.iter().map(|item| item.0.clone()).collect(),
            state.args.not_found_ttl_days,
        )
        .await
        .unwrap_or_default();
    let statuses: std::collections::HashMap<_, _> = statuses.into_iter().collect();
    let mut bssids: Vec<DebugBssid> = debug_inputs
        .into_iter()
        .map(|(bssid, signal_dbm, seen, is_stable)| {
            let db_status = match statuses.get(&bssid).copied() {
                Some(DebugDbStatus::Cached) => DbStatus::Cached,
                Some(DebugDbStatus::NotFound) => DbStatus::NotFound,
                Some(DebugDbStatus::Pending) => DbStatus::Pending,
                Some(DebugDbStatus::New) | None => DbStatus::New,
            };
            DebugBssid {
                bssid,
                signal_dbm,
                seen,
                needed: threshold,
                is_stable,
                db_status,
            }
        })
        .collect();
    // Sort: BSSIDs with a current signal first (strongest first); BSSIDs
    // without a current signal last (None sorts after Some(_) under Reverse).
    bssids.sort_by_key(|n| std::cmp::Reverse(n.signal_dbm));

    let stable_count = stable_set.len();
    let resp = DebugResponse {
        ok: true,
        v: 1,
        daemon_rev: env!("GIT_REV"),
        scan_age_ms,
        samples_in_buffer: samples,
        visible,
        stable: stable_count,
        bssids,
    };
    ok_json(&resp)
}

#[derive(Serialize)]
struct VersionResponse {
    ok: bool,
    v: u32,
    version: &'static str,
    git_rev: &'static str,
}

fn handle_version() -> String {
    let resp = VersionResponse {
        ok: true,
        v: 1,
        version: env!("CARGO_PKG_VERSION"),
        git_rev: env!("GIT_REV"),
    };
    ok_json(&resp)
}

#[derive(Serialize)]
struct HistoryResponse {
    ok: bool,
    v: u32,
    /// RFC3339 inclusive start of the queried range.
    from: String,
    /// RFC3339 inclusive end of the queried range.
    to: String,
    /// Stay-point segments in ascending time order.
    segments: Vec<crate::history::Segment>,
}

/// Async-side cancellation bridge for blocking history work. Dropping an
/// async request cannot abort `spawn_blocking`; it sets this flag instead, and
/// TASK-0087's cancellable O(N) segmentation checks it cooperatively.
struct HistoryCancellationGuard {
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    armed: bool,
}

impl HistoryCancellationGuard {
    fn new(cancelled: Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self {
            cancelled,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for HistoryCancellationGuard {
    fn drop(&mut self) {
        if self.armed {
            self.cancelled
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }
}

/// Spawn the complete CPU half of history processing. Serialization is kept
/// in the same admitted blocking closure as segmentation so neither can stall
/// a Tokio worker. The injected `before_work` seam is a per-call deterministic
/// test gate; production passes an empty closure (no globals or hooks).
struct HistoryWork {
    fixes: Vec<crate::db::FixRow>,
    from: String,
    to: String,
    dist_threshold_m: f64,
    min_duration_secs: i64,
    shutdown: tokio_util::sync::CancellationToken,
    lease: Arc<tokio::sync::OwnedSemaphorePermit>,
}

fn spawn_history_work<B>(
    work: HistoryWork,
    before_work: B,
) -> (tokio::task::JoinHandle<String>, HistoryCancellationGuard)
where
    B: FnOnce(&dyn Fn() -> bool) + Send + 'static,
{
    let HistoryWork {
        fixes,
        from,
        to,
        dist_threshold_m,
        min_duration_secs,
        shutdown,
        lease,
    } = work;
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let blocking_cancelled = Arc::clone(&cancelled);
    let guard = HistoryCancellationGuard::new(cancelled);
    let join = tokio::task::spawn_blocking(move || {
        // This clone keeps admission until the closure ACTUALLY exits, even if
        // the async waiter is force-aborted while blocking work is detached.
        let _blocking_lease = lease;
        let is_cancelled = || {
            blocking_cancelled.load(std::sync::atomic::Ordering::Acquire) || shutdown.is_cancelled()
        };
        if is_cancelled() {
            return error_json("history: cancelled");
        }
        before_work(&is_cancelled);

        let segments = match crate::history::segment_fixes_cancellable(
            &fixes,
            dist_threshold_m,
            min_duration_secs,
            is_cancelled,
        ) {
            crate::history::SegmentFixesOutcome::Complete(segments) => segments,
            crate::history::SegmentFixesOutcome::Cancelled => {
                return error_json("history: cancelled");
            }
        };
        if blocking_cancelled.load(std::sync::atomic::Ordering::Acquire) || shutdown.is_cancelled()
        {
            return error_json("history: cancelled");
        }

        ok_json(&HistoryResponse {
            ok: true,
            v: 1,
            from,
            to,
            segments,
        })
    });
    (join, guard)
}

async fn handle_history(
    range: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
) -> CommandResponse {
    // Resolve range: either `range="7d"` shorthand OR explicit from/to RFC3339.
    // Mutually exclusive. If neither is provided, default to the last 7 days.
    let (from, to) = match (range, from, to) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            return error_json("history: 'range' and 'from'/'to' are mutually exclusive").into();
        }
        (Some(spec), None, None) => match crate::history::parse_range(spec) {
            Ok(r) => r,
            Err(e) => return error_json(&format!("history: invalid range: {e}")).into(),
        },
        (None, Some(f), Some(t)) => {
            let f = match chrono::DateTime::parse_from_rfc3339(f) {
                Ok(d) => d.with_timezone(&chrono::Utc),
                Err(e) => {
                    return error_json(&format!("history: invalid 'from' timestamp: {e}")).into();
                }
            };
            let t = match chrono::DateTime::parse_from_rfc3339(t) {
                Ok(d) => d.with_timezone(&chrono::Utc),
                Err(e) => {
                    return error_json(&format!("history: invalid 'to' timestamp: {e}")).into();
                }
            };
            if f >= t {
                return error_json("history: 'from' must be before 'to'").into();
            }
            (f, t)
        }
        (None, None, None) => {
            crate::history::parse_range("7d").expect("hardcoded default range '7d' must parse")
        }
        _ => {
            return error_json("history: provide either 'range' or both 'from' and 'to'").into();
        }
    };

    // Never queue history waiters behind the expensive request: they would
    // consume connection slots and let a burst starve cheap commands.
    let lease = match Arc::clone(&state.history_slot).try_acquire_owned() {
        Ok(lease) => Arc::new(lease),
        Err(_) => return error_json("history: busy; retry later").into(),
    };

    let fixes = match state
        .db
        .get_fixes_in_range_guarded(from.to_rfc3339(), to.to_rfc3339(), Arc::clone(&lease))
        .await
    {
        Ok(v) => v,
        Err(e) => {
            return CommandResponse::history(
                error_json(&format!("history: db error: {e}")),
                lease,
                ticket,
            );
        }
    };

    let from = from.to_rfc3339();
    let to = to.to_rfc3339();
    let (join, mut cancellation) = spawn_history_work(
        HistoryWork {
            fixes,
            from,
            to,
            dist_threshold_m: state.args.history_segment_distance_m as f64,
            min_duration_secs: state.args.history_segment_min_duration_secs as i64,
            shutdown: state.shutdown.clone(),
            lease: Arc::clone(&lease),
        },
        |_| {},
    );
    let body = match join.await {
        Ok(body) => body,
        Err(error) => {
            warn!("history blocking worker failed: {error}");
            error_json("history: processing failed")
        }
    };
    cancellation.disarm();
    CommandResponse::history(body, lease, ticket)
}

async fn perform_privacy_wipe(
    state: Arc<DaemonState>,
    durable_admission: Arc<WipeAdmission>,
) -> Result<(
    crate::db::PrivacyWipeCounts,
    crate::privacy::PrivacyWipeLease,
)> {
    perform_privacy_wipe_core(state, durable_admission, || {}, None).await
}

/// Shared production/test core. The hook is placed at the exact committed-DB,
/// pre-clear boundary so the regression test can hold that real ordering open.
#[cfg(test)]
async fn perform_privacy_wipe_with_hook<H>(
    state: Arc<DaemonState>,
    durable_admission: Arc<WipeAdmission>,
    after_db_commit: H,
) -> Result<(
    crate::db::PrivacyWipeCounts,
    crate::privacy::PrivacyWipeLease,
)>
where
    H: FnOnce() + Send + 'static,
{
    perform_privacy_wipe_core(state, durable_admission, after_db_commit, None).await
}

async fn perform_privacy_wipe_core<H>(
    state: Arc<DaemonState>,
    durable_admission: Arc<WipeAdmission>,
    after_db_commit: H,
    queued: Option<tokio::sync::oneshot::Sender<()>>,
) -> Result<(
    crate::db::PrivacyWipeCounts,
    crate::privacy::PrivacyWipeLease,
)>
where
    H: FnOnce() + Send + 'static,
{
    let lease = state.privacy.begin_wipe().await;
    let durable_guard = DurableWipeGuard::new(lease, durable_admission);
    let cleanup_state = Arc::clone(&state);
    // The clear+epoch continuation is owned by the same durable, fail-stop
    // actor command as the SQLite transaction. Cancelling either async waiter
    // cannot strand a committed DB wipe with stale managed memory. A normal DB
    // error skips the continuation and drops the exclusive lease unchanged; a
    // panic after commit terminates the process instead of recovering the actor
    // into a persisted/in-memory split.
    let (counts, (), durable_guard) = state
        .db
        .wipe_privacy_data_then(
            durable_guard,
            move |guard| {
                after_db_commit();

                // Clear every managed store while the epoch is still old. ticket()
                // is deliberately lock-free, so a ticket captured anywhere in the
                // commit/clear interval must remain old and be rejected after the
                // final epoch advance. Advancing first would let scan/debug capture
                // the new epoch while still reading pre-clear BSSID/SSID state.
                cleanup_state.debouncer.blocking_lock().clear();
                *cleanup_state.last_fix.blocking_lock() = None;
                cleanup_state
                    .address_cache
                    .lock()
                    .unwrap_or_else(|error| {
                        warn!("address cache mutex poisoned during wipe — recovering");
                        error.into_inner()
                    })
                    .clear();
                cleanup_state.geocode.clear_for_wipe();
                cleanup_state
                    .inflight
                    .lock()
                    .unwrap_or_else(|error| {
                        warn!("inflight mutex poisoned during wipe — recovering");
                        error.into_inner()
                    })
                    .clear();
                guard.lease.advance_epoch();
            },
            queued,
        )
        .await?;

    Ok((counts, durable_guard.lease))
}

/// Start an accepted wipe outside its connection handler. The set is
/// single-flight by admission and gets its own bounded shutdown drain. Once
/// enqueued, the durable DB actor command retains both admission and the
/// clear+epoch continuation even if this async worker is aborted.
fn spawn_privacy_wipe(
    state: &Arc<DaemonState>,
    admission: Arc<WipeAdmission>,
    queued: Option<tokio::sync::oneshot::Sender<()>>,
) -> tokio::sync::oneshot::Receiver<
    Result<(
        crate::db::PrivacyWipeCounts,
        crate::privacy::PrivacyWipeLease,
    )>,
> {
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let worker_state = Arc::clone(state);
    let mut wipes = state
        .privacy_wipes
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    while wipes.try_join_next().is_some() {}
    wipes.spawn(async move {
        let result = if queued.is_some() {
            perform_privacy_wipe_core(worker_state, admission, || {}, queued).await
        } else {
            perform_privacy_wipe(worker_state, admission).await
        };
        let _ = result_tx.send(result);
    });
    result_rx
}

async fn handle_wipe(
    confirmation: Option<&str>,
    state: &Arc<DaemonState>,
    transport: TransportKind,
) -> CommandResponse {
    handle_wipe_observing_enqueue(confirmation, state, transport, None).await
}

/// The optional signal fires only after the durable writer command owns the
/// wipe continuation. Production passes `None`; cancellation tests use the
/// signal instead of scheduler yields to prove they abort after enqueue.
async fn handle_wipe_observing_enqueue(
    confirmation: Option<&str>,
    state: &Arc<DaemonState>,
    transport: TransportKind,
    queued: Option<tokio::sync::oneshot::Sender<()>>,
) -> CommandResponse {
    if transport != TransportKind::Unix {
        return error_json(WIPE_TCP_ERROR).into();
    }
    if confirmation != Some(WIPE_CONFIRMATION) {
        return error_json(WIPE_CONFIRMATION_ERROR).into();
    }
    let admission = match Arc::clone(&state.wipe_slot).try_acquire_owned() {
        Ok(permit) => Arc::new(WipeAdmission { _permit: permit }),
        Err(_) => return error_json(WIPE_BUSY_ERROR).into(),
    };

    // The dedicated worker owns the exclusive lease and complete
    // DB->memory->epoch sequence. If this connection disappears (including a
    // shutdown force-abort), the wipe still finishes instead of committing
    // SQLite and leaving stale managed memory behind.
    let result = spawn_privacy_wipe(state, Arc::clone(&admission), queued);
    match result.await {
        Ok(Ok((counts, lease))) => CommandResponse::wipe(
            ok_json(&WipeResponse {
                ok: true,
                v: 1,
                aps_deleted: counts.aps_deleted,
                pending_deleted: counts.pending_deleted,
                not_found_deleted: counts.not_found_deleted,
                fixes_deleted: counts.fixes_deleted,
                last_fix_deleted: counts.last_fix_deleted,
            }),
            lease,
            admission,
        ),
        Ok(Err(error)) => {
            CommandResponse::wipe_error(error_json(&format!("wipe: db error: {error}")), admission)
        }
        Err(_) => CommandResponse::wipe_error(error_json(WIPE_RESULT_UNKNOWN_ERROR), admission),
    }
}

async fn handle_stats(state: &Arc<DaemonState>) -> String {
    let inflight = state
        .inflight
        .lock()
        .map(|s| s.len())
        .unwrap_or_else(|e| e.into_inner().len());
    let db_write_failures = state
        .db_write_failures
        .load(std::sync::atomic::Ordering::Relaxed);
    let api_counter_corruption_denials = state
        .api_counter_corruption_denials
        .load(std::sync::atomic::Ordering::Relaxed);
    let orphan_tasks_aborted = state
        .orphan_tasks_aborted
        .load(std::sync::atomic::Ordering::Relaxed);
    let stats = state.db.stats().await.unwrap_or_default();
    let resp = StatsResponse {
        ok: true,
        v: 1,
        cached_aps: stats.cached_aps,
        pending_aps: stats.pending_aps,
        not_found_aps: stats.not_found_aps,
        db_size_bytes: stats.db_size_bytes,
        api_calls_today: stats.api_calls_today,
        api_counter_corruption_denials,
        inflight,
        db_write_failures,
        orphan_tasks_aborted,
    };
    ok_json(&resp)
}

// Address-cache tests moved to server/address_cache.rs alongside the impl.

/// Build a minimal, side-effect-free `DaemonState` backed by an in-memory
/// SQLite DB, shared by the in-file test modules. The network clients construct
/// HTTP clients only (no I/O at construction), so this is cheap and hermetic.
#[cfg(test)]
pub(crate) fn test_state() -> std::sync::Arc<DaemonState> {
    test_state_with_nominatim(crate::nominatim::NominatimClient::new())
}

#[cfg(test)]
pub(crate) fn test_state_with_nominatim(
    nominatim: crate::nominatim::NominatimClient,
) -> std::sync::Arc<DaemonState> {
    let (db_runtime, db, test_db) = crate::db::DbRuntime::open_test().unwrap();
    test_state_with_db(db_runtime, db, test_db, nominatim)
}

#[cfg(test)]
fn test_state_with_db(
    db_runtime: crate::db::DbRuntime,
    db: crate::db::DbHandle,
    test_db: crate::db::Database,
    nominatim: crate::nominatim::NominatimClient,
) -> std::sync::Arc<DaemonState> {
    use clap::Parser;
    let args = crate::config::Args::parse_from(["whereamid"]);
    let shutdown = tokio_util::sync::CancellationToken::new();
    std::sync::Arc::new(DaemonState {
        db,
        _db_runtime: Some(db_runtime),
        test_db: std::sync::Mutex::new(test_db),
        debouncer: tokio::sync::Mutex::new(crate::debounce::Debouncer::new(
            args.debounce_window,
            args.debounce_threshold,
        )),
        privacy: crate::privacy::PrivacyCoordinator::new(),
        wigle: crate::wigle::WigleClient::new("", ""),
        apple: crate::apple::AppleClient::new(),
        nominatim,
        geocode: geocode::GeocodeScheduler::new(shutdown.clone()),
        last_fix: tokio::sync::Mutex::new(None),
        inflight: std::sync::Mutex::new(std::collections::HashMap::new()),
        address_cache: std::sync::Mutex::new(AddressCache::new()),
        db_write_failures: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        api_counter_corruption_denials: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        connection_slots: Arc::new(tokio::sync::Semaphore::new(args.max_connections)),
        history_slot: Arc::new(tokio::sync::Semaphore::new(1)),
        wipe_slot: Arc::new(tokio::sync::Semaphore::new(1)),
        shutdown,
        tasks: std::sync::Mutex::new(tokio::task::JoinSet::new()),
        privacy_wipes: std::sync::Mutex::new(tokio::task::JoinSet::new()),
        orphan_tasks_aborted: std::sync::atomic::AtomicU64::new(0),
        args,
    })
}

#[cfg(test)]
mod stats_tests {
    use super::*;

    #[test]
    fn protocol_documents_api_counter_corruption_denial_semantics() {
        let protocol = include_str!("../../docs/protocol.md");
        for marker in [
            "\"api_counter_corruption_denials\": 0",
            "cumulative executed WiGLE reservation",
            "distinct from normal quota",
            "resets on restart",
            "api_calls_today` can render its fallback `0`",
            "does not change protocol version `v: 1`",
        ] {
            assert!(
                protocol.contains(marker),
                "protocol is missing semantic marker {marker:?}"
            );
        }
    }

    #[tokio::test]
    async fn stats_wire_reports_corruption_denials_while_counter_falls_back_to_zero() {
        let state = test_state();
        {
            let db = lock_db(&state);
            db.set_metadata("api_calls_today", "malformed").unwrap();
        }
        state
            .api_counter_corruption_denials
            .store(3, std::sync::atomic::Ordering::Relaxed);

        let response: serde_json::Value =
            serde_json::from_str(&handle_stats(&state).await).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["v"], 1);
        assert_eq!(response["api_calls_today"], 0);
        assert_eq!(response["api_counter_corruption_denials"], 3);
    }
}

#[cfg(test)]
mod last_fix_concurrency_tests {
    //! TASK-0046 AC#3: race address backfill against a newer fix on one real
    //! DaemonState. Synchronisation is at the exact persistence boundary, not
    //! a timing sleep, so moving the DB command outside the last_fix guard makes
    //! the guard-retention assertion fail deterministically.
    use super::*;
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_backfill_and_new_fix_keep_memory_and_db_consistent() {
        const OLD_LAT: f64 = 55.6684;
        const OLD_LON: f64 = 12.5541;
        const NEW_LAT: f64 = 55.6761;
        const NEW_LON: f64 = 12.5683;

        let state = test_state();
        // Equal timestamps are intentional: task-0100's monotonic UPSERT must
        // not mask this guard-scoping regression by rejecting a stale row.
        let at = chrono::DateTime::parse_from_rfc3339("2026-07-05T00:00:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let old_fix = LastFix {
            lat: OLD_LAT,
            lon: OLD_LON,
            accuracy_m: 20.0,
            address: None,
            at,
            sources: 3,
        };
        let old_row: crate::db::LastFixRow = (&old_fix).into();
        {
            let mut last = state.last_fix.lock().await;
            *last = Some(old_fix);
        }
        state.db.set_last_fix(old_row).await.unwrap();

        // Occupy the real writer actor with a deterministic no-op command so
        // the backfill queues at its production persistence boundary.
        let (db_held_tx, db_held_rx) = std::sync::mpsc::channel();
        let (release_db_tx, release_db_rx) = std::sync::mpsc::channel();
        let blocker_db = state.db.clone();
        let blocker = tokio::spawn(async move {
            blocker_db
                .hold_writer_until(db_held_tx, release_db_rx)
                .await
        });
        db_held_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("writer actor blocker must start");

        // Signal from the injected *real persistence closure* immediately
        // before it asks for the blocked DB. The helper contract requires the
        // last_fix guard to remain alive at this exact point.
        let (persist_reached_tx, persist_reached_rx) = tokio::sync::oneshot::channel();
        let backfill_state = Arc::clone(&state);
        let backfill = tokio::spawn(async move {
            backfill_last_fix_address_with(
                &backfill_state,
                backfill_state.privacy.ticket(),
                OLD_LAT,
                OLD_LON,
                "old address",
                move |db, row| async move {
                    let _ = persist_reached_tx.send(());
                    db.set_last_fix(row).await
                },
            )
            .await
        });

        let persist_reached = matches!(
            tokio::time::timeout(Duration::from_secs(2), persist_reached_rx).await,
            Ok(Ok(()))
        );
        let backfill_retained_last_fix = persist_reached && state.last_fix.try_lock().is_err();

        // This is the handle_locate-side operation on the SAME DaemonState.
        // It must wait for the backfill's last_fix guard, then atomically make
        // both memory and disk point at NEW.
        let replacement_state = Arc::clone(&state);
        let new_fix = LastFix {
            lat: NEW_LAT,
            lon: NEW_LON,
            accuracy_m: 8.0,
            address: None,
            at,
            sources: 7,
        };
        let replacement = tokio::spawn(async move {
            persist_last_fix_and_history(
                &replacement_state,
                replacement_state.privacy.ticket(),
                new_fix,
            )
            .await;
        });

        // Cleanup precedes every assertion: a failing discriminator must not
        // strand the blocker thread or either Tokio task.
        let _ = release_db_tx.send(());
        blocker
            .await
            .expect("DB blocker task panicked")
            .expect("DB blocker failed");
        let joined = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(backfill, replacement)
        })
        .await;

        assert!(persist_reached, "backfill never reached its DB boundary");
        assert!(
            backfill_retained_last_fix,
            "last_fix guard was released before backfill persistence (the old task-0046 race)"
        );
        let (backfill_join, replacement_join) =
            joined.expect("concurrent last_fix operations exceeded the 2s watchdog");
        assert!(
            backfill_join
                .expect("backfill task panicked")
                .expect("backfill persistence failed"),
            "matching old fix should have been backfilled"
        );
        replacement_join.expect("replacement task panicked");

        let memory_row = {
            let last = state.last_fix.lock().await;
            crate::db::LastFixRow::from(last.as_ref().expect("last_fix missing in memory"))
        };
        let disk_row = state
            .db
            .get_last_fix()
            .await
            .unwrap()
            .expect("last_fix missing on disk");

        assert_eq!(memory_row.lat, NEW_LAT);
        assert_eq!(memory_row.lon, NEW_LON);
        assert_eq!(memory_row.accuracy_m, 8.0);
        assert_eq!(memory_row.address, None);
        assert_eq!(memory_row.sources, 7);
        assert_eq!(disk_row.lat, memory_row.lat);
        assert_eq!(disk_row.lon, memory_row.lon);
        assert_eq!(disk_row.accuracy_m, memory_row.accuracy_m);
        assert_eq!(disk_row.address, memory_row.address);
        assert_eq!(disk_row.at_rfc3339, memory_row.at_rfc3339);
        assert_eq!(disk_row.sources, memory_row.sources);
    }
}

#[cfg(test)]
mod wire_enum_tests {
    //! task-0062: wire-format round-trip for the typed enums that
    //! previously crossed the protocol as raw strings. These tests
    //! pin the on-the-wire spelling so a future variant rename
    //! (Rust-side) cannot silently break clients.
    use super::*;

    #[test]
    fn db_status_serializes_to_documented_strings() {
        assert_eq!(
            serde_json::to_string(&DbStatus::Cached).unwrap(),
            "\"cached\""
        );
        assert_eq!(
            serde_json::to_string(&DbStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&DbStatus::NotFound).unwrap(),
            "\"not_found\""
        );
        assert_eq!(serde_json::to_string(&DbStatus::New).unwrap(), "\"new\"");
    }

    #[test]
    fn db_status_round_trip() {
        for v in [
            DbStatus::Cached,
            DbStatus::Pending,
            DbStatus::NotFound,
            DbStatus::New,
        ] {
            let s = serde_json::to_string(&v).unwrap();
            let back: DbStatus = serde_json::from_str(&s).unwrap();
            assert_eq!(back, v);
        }
    }

    #[test]
    fn provenance_serializes_to_documented_strings() {
        assert_eq!(serde_json::to_string(&Provenance::Api).unwrap(), "\"api\"");
        assert_eq!(
            serde_json::to_string(&Provenance::Cache).unwrap(),
            "\"cache\""
        );
        assert_eq!(
            serde_json::to_string(&Provenance::NotFound).unwrap(),
            "\"not_found\""
        );
        // task-0194: new wire value.
        assert_eq!(
            serde_json::to_string(&Provenance::Unresolved).unwrap(),
            "\"unresolved\""
        );
    }

    #[test]
    fn provenance_round_trip() {
        for v in [
            Provenance::Api,
            Provenance::Cache,
            Provenance::NotFound,
            Provenance::Unresolved,
        ] {
            let s = serde_json::to_string(&v).unwrap();
            let back: Provenance = serde_json::from_str(&s).unwrap();
            assert_eq!(back, v);
        }
    }

    #[test]
    fn resolve_result_emits_source_field_name() {
        // Provenance is internal; the JSON field MUST stay `source` for
        // backward-compatible clients (curl scripts, older CLIs).
        let r = ResolveResult {
            bssid: "aa:bb:cc:dd:ee:ff".to_string(),
            lat: Some(1.0),
            lon: Some(2.0),
            ssid: Some("x".to_string()),
            provenance: Provenance::Cache,
        };
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v.get("source").and_then(|x| x.as_str()), Some("cache"));
        assert!(
            v.get("provenance").is_none(),
            "internal field name must not leak onto the wire"
        );
    }

    #[test]
    fn unknown_db_status_string_is_rejected() {
        // We deliberately do NOT add #[serde(other)] — an unknown variant
        // on the wire is a protocol mismatch and we want it loud, not
        // silently mapped to a fallback. Newer daemons emitting a new
        // variant will need a coordinated client bump (or a future
        // task to add a tagged-other escape hatch).
        let r: Result<DbStatus, _> = serde_json::from_str("\"unrecognised\"");
        assert!(r.is_err(), "deserializing unknown db_status must fail");
    }
}

#[cfg(test)]
mod dispatch_tests {
    //! Full-dispatch integration tests (Request -> dispatch_command -> handler
    //! -> JSON) against a real in-memory DaemonState. Covers history overflow
    //! (task-0184), resolve provenance mapping (task-0194), and poisoned
    //! SQLite coordinates at the resolve serve boundary (task-0238).
    use super::test_state;
    use super::*;

    const WIPE_FAIL_STOP_CHILD_ENV: &str = "WHEREAMID_WIPE_FAIL_STOP_TEST_CHILD";
    const WIPE_FAIL_STOP_DB_ENV: &str = "WHEREAMID_WIPE_FAIL_STOP_TEST_DB";
    const WIPE_FAIL_STOP_RESUMED_ENV: &str = "WHEREAMID_WIPE_FAIL_STOP_TEST_RESUMED";
    const WIPE_FAIL_STOP_READER_WAITING_ENV: &str = "WHEREAMID_WIPE_FAIL_STOP_READER_WAITING";
    const WIPE_FAIL_STOP_READER_ACQUIRED_ENV: &str = "WHEREAMID_WIPE_FAIL_STOP_READER_ACQUIRED";
    const WIPE_FAIL_STOP_CHILD_TEST: &str =
        "server::dispatch_tests::wipe_post_commit_panic_fail_stop_child";

    fn wait_for_child_marker(
        child: &mut std::process::Child,
        path: &std::path::Path,
        budget: std::time::Duration,
    ) -> bool {
        let deadline = std::time::Instant::now() + budget;
        loop {
            if path.exists() {
                return true;
            }
            if child.try_wait().unwrap().is_some() || std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn first_resolve_result(resp: &str) -> serde_json::Value {
        let v: serde_json::Value =
            serde_json::from_str(resp).expect("dispatch must return valid JSON");
        assert_eq!(
            v.get("ok").and_then(|b| b.as_bool()),
            Some(true),
            "resolve must succeed, got {resp}"
        );
        let results = v["results"]
            .as_array()
            .unwrap_or_else(|| panic!("resolve results missing in {resp}"));
        assert_eq!(results.len(), 1, "expected one resolve result in {resp}");
        results[0].clone()
    }

    /// Extract `results[0].source` from a resolve dispatch response.
    fn resolve_source(resp: &str) -> String {
        let result = first_resolve_result(resp);
        result["source"]
            .as_str()
            .unwrap_or_else(|| panic!("results[0].source missing in {resp}"))
            .to_string()
    }

    fn cache_ap(state: &Arc<DaemonState>, bssid: &str, lat: f64, lon: f64) {
        let ap = crate::db::ApInfo {
            bssid: bssid.to_string(),
            ssid: Some("test-ap".to_string()),
            lat,
            lon,
            encryption: None,
            channel: None,
            frequency: None,
            city: None,
            country: None,
            source: crate::db::Source::Manual,
        };
        lock_db(state).upsert_ap(&ap).unwrap();
    }

    async fn seed_wipe_state(state: &Arc<DaemonState>) {
        cache_ap(state, "aa:bb:cc:dd:ee:01", 55.0, 12.0);
        {
            let db = lock_db(state);
            db.insert_pending("pending", Some("private-ssid"), None, None, Some(-40))
                .unwrap();
            db.insert_not_found("not-found").unwrap();
            db.insert_fix("2026-01-01T00:00:00+00:00", 55.0, 12.0, 5.0, 1)
                .unwrap();
            db.set_last_fix(&crate::db::LastFixRow {
                lat: 55.0,
                lon: 12.0,
                accuracy_m: 5.0,
                address: Some("private address".to_string()),
                at_rfc3339: "2026-01-01T00:00:00+00:00".to_string(),
                sources: 1,
            })
            .unwrap();
        }
        let mut sample = std::collections::HashMap::new();
        sample.insert(
            "aa:bb:cc:dd:ee:01".to_string(),
            crate::debounce::ScanEntry {
                signal_dbm: -40,
                ssid: Some("private-ssid".to_string()),
                channel: Some(6),
            },
        );
        state.debouncer.lock().await.push_scan(sample);
        *state.last_fix.lock().await = Some(LastFix {
            lat: 55.0,
            lon: 12.0,
            accuracy_m: 5.0,
            address: Some("private address".to_string()),
            at: chrono::Utc::now(),
            sources: 1,
        });
        state
            .address_cache
            .lock()
            .unwrap()
            .insert(55.0, 12.0, "private address".to_string());
        state
            .geocode
            .observe(55.0, 12.0, true, state.privacy.ticket());
        state
            .inflight
            .lock()
            .unwrap()
            .insert("aa:bb:cc:dd:ee:01".to_string(), state.privacy.ticket());
    }

    /// Subprocess-only half of the fail-stop regression below. With the old
    /// generic actor panic policy this continued after `perform_privacy_wipe`
    /// returned an error, observed a reusable actor plus stale memory, and
    /// wrote the resumed marker. The fail-stop command must terminate the
    /// entire process from the actor thread before any of that can execute.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wipe_post_commit_panic_fail_stop_child() {
        if std::env::var_os(WIPE_FAIL_STOP_CHILD_ENV).as_deref() != Some(std::ffi::OsStr::new("1"))
        {
            return;
        }
        let db_path = std::path::PathBuf::from(
            std::env::var_os(WIPE_FAIL_STOP_DB_ENV).expect("child DB path"),
        );
        let resumed_path = std::path::PathBuf::from(
            std::env::var_os(WIPE_FAIL_STOP_RESUMED_ENV).expect("child resumed marker path"),
        );
        let reader_waiting_path = std::path::PathBuf::from(
            std::env::var_os(WIPE_FAIL_STOP_READER_WAITING_ENV)
                .expect("child reader-waiting marker path"),
        );
        let reader_acquired_path = std::path::PathBuf::from(
            std::env::var_os(WIPE_FAIL_STOP_READER_ACQUIRED_ENV)
                .expect("child reader-acquired marker path"),
        );
        let (db_runtime, db) = crate::db::DbRuntime::open(&db_path).await.unwrap();
        let state = test_state_with_db(
            db_runtime,
            db,
            crate::db::Database::open_memory().unwrap(),
            crate::nominatim::NominatimClient::new(),
        );
        *state.last_fix.lock().await = Some(LastFix {
            lat: 55.0,
            lon: 12.0,
            accuracy_m: 5.0,
            address: Some("stale private address".to_string()),
            at: chrono::Utc::now(),
            sources: 1,
        });
        let admission = Arc::new(WipeAdmission {
            _permit: Arc::clone(&state.wipe_slot).try_acquire_owned().unwrap(),
        });

        let old_ticket = state.privacy.ticket();
        let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
        let wipe_state = Arc::clone(&state);
        let wipe = tokio::spawn(async move {
            perform_privacy_wipe_core(
                wipe_state,
                admission,
                || panic!("injected panic after privacy wipe commit"),
                Some(queued_tx),
            )
            .await
        });
        queued_rx
            .await
            .expect("fail-stop wipe must enqueue its guarded command");

        let reader_state = Arc::clone(&state);
        let reader = tokio::spawn(async move {
            let lease = reader_state
                .privacy
                .read_observing_wait(old_ticket, || {
                    let _ = std::fs::write(reader_waiting_path, b"waiting");
                })
                .await;
            let _ = std::fs::write(
                reader_acquired_path,
                if lease.is_some() { "current" } else { "stale" },
            );
        });

        let wipe = wipe.await;
        let reader = reader.await;

        // Reaching this code is the defect: the actor recovered from an
        // irrecoverable post-COMMIT panic and the process could keep serving.
        let actor_reusable = state.db.stats().await.is_ok();
        let stale_memory = state.last_fix.lock().await.is_some();
        std::fs::write(
            resumed_path,
            format!(
                "wipe_join_error={} reader_join_error={} actor_reusable={actor_reusable} \
                 stale_memory={stale_memory}",
                wipe.is_err(),
                reader.is_err()
            ),
        )
        .unwrap();
    }

    #[test]
    fn post_commit_wipe_panic_terminates_process_after_durable_commit() {
        let dir = std::env::temp_dir().join(format!(
            "whereamid-wipe-fail-stop-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("whereami.db");
        let resumed_path = dir.join("process-resumed-serving");
        let fatal_entered_path = dir.join("fatal-entered");
        let fatal_release_path = dir.join("fatal-release");
        let reader_waiting_path = dir.join("reader-waiting");
        let reader_acquired_path = dir.join("reader-acquired");
        let guard_dropped_path = dir.join("wipe-guard-dropped");
        {
            let db = crate::db::Database::open(&db_path).unwrap();
            db.upsert_ap(&crate::db::ApInfo {
                bssid: "aa:bb:cc:dd:ee:ff".to_string(),
                ssid: Some("private-ssid".to_string()),
                lat: 55.0,
                lon: 12.0,
                encryption: None,
                channel: None,
                frequency: None,
                city: None,
                country: None,
                source: crate::db::Source::Manual,
            })
            .unwrap();
            db.insert_pending("pending", Some("private-ssid"), None, None, Some(-40))
                .unwrap();
            db.insert_not_found("not-found").unwrap();
            db.insert_fix("2026-01-01T00:00:00+00:00", 55.0, 12.0, 5.0, 1)
                .unwrap();
            db.set_last_fix(&crate::db::LastFixRow {
                lat: 55.0,
                lon: 12.0,
                accuracy_m: 5.0,
                address: Some("private address".to_string()),
                at_rfc3339: "2026-01-01T00:00:00+00:00".to_string(),
                sources: 1,
            })
            .unwrap();
        }

        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(WIPE_FAIL_STOP_CHILD_TEST)
            .arg("--nocapture")
            .arg("--test-threads=1")
            .env(WIPE_FAIL_STOP_CHILD_ENV, "1")
            .env(WIPE_FAIL_STOP_DB_ENV, &db_path)
            .env(WIPE_FAIL_STOP_RESUMED_ENV, &resumed_path)
            .env(WIPE_FAIL_STOP_READER_WAITING_ENV, &reader_waiting_path)
            .env(WIPE_FAIL_STOP_READER_ACQUIRED_ENV, &reader_acquired_path)
            .env(WIPE_GUARD_DROP_MARKER_ENV, &guard_dropped_path)
            .env(crate::db::FAIL_STOP_TEST_ENTERED_ENV, &fatal_entered_path)
            .env(crate::db::FAIL_STOP_TEST_RELEASE_ENV, &fatal_release_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();

        let fatal_entered = wait_for_child_marker(
            &mut child,
            &fatal_entered_path,
            std::time::Duration::from_secs(10),
        );
        let reader_waiting = wait_for_child_marker(
            &mut child,
            &reader_waiting_path,
            std::time::Duration::from_secs(10),
        );
        let reader_acquired_while_paused = reader_acquired_path.exists();
        let guard_dropped_while_paused = guard_dropped_path.exists();
        let resumed_while_paused = resumed_path.exists();

        // Release the fatal path even when a preceding marker was missing, so
        // the parent always has a bounded path to collect/kill the subprocess.
        std::fs::write(&fatal_release_path, b"release").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let timed_out = loop {
            if child.try_wait().unwrap().is_some() {
                break false;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                break true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let status = output.status;
        let resumed = resumed_path.exists();
        let reader_acquired = reader_acquired_path.exists();
        let guard_dropped = guard_dropped_path.exists();

        // Reopen after the abrupt process exit. Seeing every table empty proves
        // the injected panic ran after SQLite COMMIT, not before it.
        let persisted_empty = {
            let db = crate::db::Database::open(&db_path).unwrap();
            db.cached_ap_count().unwrap() == 0
                && db.pending_ap_count().unwrap() == 0
                && db.not_found_ap_count().unwrap() == 0
                && db.get_fixes_in_range("0000", "9999").unwrap().is_empty()
                && db.get_last_fix().unwrap().is_none()
        };
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(
            fatal_entered,
            "child never reached the deterministically paused fatal path; stderr:\n{stderr}"
        );
        assert!(
            reader_waiting,
            "privacy reader never reported Poll::Pending behind the wipe guard; stderr:\n{stderr}"
        );
        assert!(
            !reader_acquired_while_paused,
            "privacy reader acquired while fatal handling was paused"
        );
        assert!(
            !guard_dropped_while_paused,
            "privacy guard dropped before fail-stop termination"
        );
        assert!(
            !resumed_while_paused,
            "child resumed serving while fatal handling was paused"
        );
        assert!(!timed_out, "fail-stop child did not terminate within 10s");
        assert_eq!(
            status.code(),
            Some(70),
            "post-commit panic must exit via the fail-stop policy; stderr:\n{stderr}"
        );
        assert!(
            stderr.contains("privacy wipe committed-state continuation")
                && stderr.contains("exiting 70"),
            "fatal diagnostic must identify the fail-stop operation; stderr:\n{stderr}"
        );
        assert!(
            !resumed,
            "child resumed after irrecoverable post-commit panic"
        );
        assert!(
            !reader_acquired,
            "privacy reader acquired before fail-stop process exit"
        );
        assert!(
            !guard_dropped,
            "privacy guard destructor ran before fail-stop process exit"
        );
        assert!(persisted_empty, "wipe must commit before fail-stop exit");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn commit_to_clear_tickets_cannot_publish_stale_scan_or_debug_bodies() {
        let state = test_state();
        seed_wipe_state(&state).await;
        let (committed_tx, committed_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let admission = Arc::new(WipeAdmission {
            _permit: Arc::clone(&state.wipe_slot).try_acquire_owned().unwrap(),
        });
        let wipe_state = Arc::clone(&state);
        let wipe = tokio::spawn(async move {
            perform_privacy_wipe_with_hook(wipe_state, admission, move || {
                let _ = committed_tx.send(());
                let _ = release_rx.recv();
            })
            .await
        });
        committed_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        // The DB is committed, but the production wipe core is deliberately
        // stalled before memory clear and epoch advance. Both handlers capture
        // the lock-free current ticket and can still build a pre-clear body.
        let mut scan = dispatch_command(&Request::Scan, &state).await;
        let mut debug = dispatch_command(&Request::Debug, &state).await;
        assert!(scan.body.contains("aa:bb:cc:dd:ee:01"));
        assert!(scan.body.contains("private-ssid"));
        assert!(debug.body.contains("aa:bb:cc:dd:ee:01"));

        release_tx.send(()).unwrap();
        let (_counts, lease) = wipe.await.unwrap().unwrap();
        drop(lease);
        scan.prepare_for_write(&state).await;
        debug.prepare_for_write(&state).await;

        let stale = error_json(STALE_AFTER_WIPE_ERROR);
        assert_eq!(scan.body, stale);
        assert_eq!(debug.body, stale);
        for body in [&scan.body, &debug.body] {
            assert!(!body.contains("aa:bb:cc:dd:ee:01"));
            assert!(!body.contains("private-ssid"));
        }
        assert_eq!(state.debouncer.lock().await.sample_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_wipe_is_busy_while_first_db_is_blocked_and_response_is_held() {
        let state = test_state();
        seed_wipe_state(&state).await;
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker_db = state.db.clone();
        let blocker =
            tokio::spawn(async move { blocker_db.hold_writer_until(entered_tx, release_rx).await });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        let first_state = Arc::clone(&state);
        let first = tokio::spawn(async move {
            dispatch_command(
                &Request::Wipe {
                    confirm: Some(WIPE_CONFIRMATION.to_string()),
                },
                &first_state,
            )
            .await
        });
        while state.wipe_slot.available_permits() != 0 {
            tokio::task::yield_now().await;
        }

        let second = tokio::time::timeout(
            Duration::from_millis(100),
            dispatch_command(
                &Request::Wipe {
                    confirm: Some(WIPE_CONFIRMATION.to_string()),
                },
                &state,
            ),
        )
        .await
        .expect("overlapping wipe must be rejected promptly");
        assert_eq!(second.body, error_json(WIPE_BUSY_ERROR));

        release_tx.send(()).unwrap();
        blocker.await.unwrap().unwrap();
        let first = first.await.unwrap();
        assert!(first.body.contains(r#""ok":true"#));
        assert_eq!(
            state.wipe_slot.available_permits(),
            0,
            "accepted response must retain admission after worker completion"
        );
        let while_response_held = dispatch_command(
            &Request::Wipe {
                confirm: Some(WIPE_CONFIRMATION.to_string()),
            },
            &state,
        )
        .await;
        assert_eq!(while_response_held.body, error_json(WIPE_BUSY_ERROR));
        drop(first);
        assert_eq!(state.wipe_slot.available_permits(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocked_db_wipe_has_bounded_shutdown_wait_and_durable_cleanup() {
        let state = test_state();
        seed_wipe_state(&state).await;
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker_db = state.db.clone();
        let blocker =
            tokio::spawn(async move { blocker_db.hold_writer_until(entered_tx, release_rx).await });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
        let first_state = Arc::clone(&state);
        let first = tokio::spawn(async move {
            handle_wipe_observing_enqueue(
                Some(WIPE_CONFIRMATION),
                &first_state,
                TransportKind::Unix,
                Some(queued_tx),
            )
            .await
        });
        queued_rx
            .await
            .expect("wipe command must be durably enqueued behind the blocker");
        assert_eq!(state.wipe_slot.available_permits(), 0);
        let wipes = {
            let mut guard = state
                .privacy_wipes
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            std::mem::take(&mut *guard)
        };
        let started = std::time::Instant::now();
        let aborted =
            crate::run_privacy_wipe_shutdown_drain(Duration::from_millis(50), wipes).await;
        assert_eq!(aborted, 1);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "blocked wipe exceeded its bounded async shutdown drain"
        );
        assert_eq!(
            state.wipe_slot.available_permits(),
            0,
            "actor-owned admission must survive worker force-abort"
        );

        let unavailable_response = first.await.unwrap();
        assert_eq!(
            unavailable_response.body,
            error_json(WIPE_RESULT_UNKNOWN_ERROR)
        );
        assert_eq!(
            state.wipe_slot.available_permits(),
            0,
            "unknown-result response must retain admission through its write lifetime"
        );

        release_tx.send(()).unwrap();
        blocker.await.unwrap().unwrap();
        state.db.writer_barrier().await.unwrap();
        assert_eq!(
            state.wipe_slot.available_permits(),
            0,
            "response still owns admission after durable cleanup completes"
        );
        drop(unavailable_response);
        assert_eq!(state.wipe_slot.available_permits(), 1);
        assert_eq!(lock_db(&state).cached_ap_count().unwrap(), 0);
        assert!(state.last_fix.lock().await.is_none());
    }

    #[tokio::test]
    async fn wipe_confirmation_is_mandatory_and_success_clears_disk_and_memory() {
        let state = test_state();
        seed_wipe_state(&state).await;

        for confirm in [None, Some("wrong".to_string())] {
            let response = dispatch_command(&Request::Wipe { confirm }, &state).await;
            assert_eq!(response.body, error_json(WIPE_CONFIRMATION_ERROR));
            assert_eq!(lock_db(&state).cached_ap_count().unwrap(), 1);
        }

        let response = dispatch_command(
            &Request::Wipe {
                confirm: Some(WIPE_CONFIRMATION.to_string()),
            },
            &state,
        )
        .await;
        let json: serde_json::Value = serde_json::from_str(&response.body).unwrap();
        assert_eq!(json["ok"], true);
        for (field, expected) in [
            ("aps_deleted", 1),
            ("pending_deleted", 1),
            ("not_found_deleted", 1),
            ("fixes_deleted", 1),
            ("last_fix_deleted", 1),
        ] {
            assert_eq!(json[field], expected, "wrong count for {field}");
        }
        assert_eq!(state.wipe_slot.available_permits(), 0);
        drop(response);
        assert_eq!(state.wipe_slot.available_permits(), 1);

        {
            let db = lock_db(&state);
            assert_eq!(db.cached_ap_count().unwrap(), 0);
            assert_eq!(db.pending_ap_count().unwrap(), 0);
            assert_eq!(db.not_found_ap_count().unwrap(), 0);
            assert!(db.get_fixes_in_range("0000", "9999").unwrap().is_empty());
            assert!(db.get_last_fix().unwrap().is_none());
        }
        assert_eq!(state.debouncer.lock().await.sample_count(), 0);
        assert!(state.last_fix.lock().await.is_none());
        assert!(state
            .address_cache
            .lock()
            .unwrap()
            .get(55.0, 12.0)
            .is_none());
        assert_eq!(state.geocode.snapshot().active, 0);
        assert_eq!(state.geocode.snapshot().pending, 0);
        assert!(state.inflight.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn wipe_db_failure_rolls_back_and_leaves_managed_memory_unchanged() {
        let state = test_state();
        seed_wipe_state(&state).await;
        lock_db(&state)
            .execute_batch_for_test(
                "CREATE TRIGGER reject_server_wipe BEFORE DELETE ON fixes
                 BEGIN SELECT RAISE(ABORT, 'server wipe failure'); END;",
            )
            .unwrap();

        let response = dispatch_command(
            &Request::Wipe {
                confirm: Some(WIPE_CONFIRMATION.to_string()),
            },
            &state,
        )
        .await;
        assert!(response.body.contains("server wipe failure"));
        assert_eq!(state.wipe_slot.available_permits(), 0);
        let overlapping = dispatch_command(
            &Request::Wipe {
                confirm: Some(WIPE_CONFIRMATION.to_string()),
            },
            &state,
        )
        .await;
        assert_eq!(overlapping.body, error_json(WIPE_BUSY_ERROR));
        drop(response);
        assert_eq!(state.wipe_slot.available_permits(), 1);
        state
            .db
            .writer_barrier()
            .await
            .expect("normal wipe Result error must leave writer actor reusable");
        assert_eq!(lock_db(&state).cached_ap_count().unwrap(), 1);
        assert_eq!(state.debouncer.lock().await.sample_count(), 1);
        assert!(state.last_fix.lock().await.is_some());
        assert!(state
            .address_cache
            .lock()
            .unwrap()
            .get(55.0, 12.0)
            .is_some());
        assert_eq!(state.geocode.snapshot().pending, 1);
        assert_eq!(state.inflight.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn wipe_db_busy_leaves_persisted_and_managed_memory_unchanged() {
        let state = test_state();
        seed_wipe_state(&state).await;
        // The legacy test connection is independent from the writer actor.
        // Keeping an IMMEDIATE transaction open makes the wipe hit SQLite's
        // real busy/locked path rather than an injected SQL error.
        lock_db(&state)
            .execute_batch_for_test(
                "BEGIN IMMEDIATE;
                 INSERT INTO metadata (key, value) VALUES ('wipe-busy', 'held')
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value;",
            )
            .unwrap();

        let response = dispatch_command(
            &Request::Wipe {
                confirm: Some(WIPE_CONFIRMATION.to_string()),
            },
            &state,
        )
        .await;
        assert!(response.body.contains("wipe: db error:"));
        assert!(
            response.body.contains("locked") || response.body.contains("busy"),
            "expected SQLite busy/locked root cause, got {}",
            response.body
        );

        {
            let db = lock_db(&state);
            assert_eq!(db.cached_ap_count().unwrap(), 1);
            assert_eq!(db.pending_ap_count().unwrap(), 1);
            assert_eq!(db.not_found_ap_count().unwrap(), 1);
            assert_eq!(db.get_fixes_in_range("0000", "9999").unwrap().len(), 1);
            assert!(db.get_last_fix().unwrap().is_some());
            db.execute_batch_for_test("ROLLBACK").unwrap();
        }
        assert_eq!(state.debouncer.lock().await.sample_count(), 1);
        assert!(state.last_fix.lock().await.is_some());
        assert!(state
            .address_cache
            .lock()
            .unwrap()
            .get(55.0, 12.0)
            .is_some());
        assert_eq!(state.geocode.snapshot().pending, 1);
        assert_eq!(state.inflight.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn pre_wipe_locate_publication_cannot_restore_last_fix() {
        let state = test_state();
        let stale_ticket = state.privacy.ticket();
        let response = dispatch_command(
            &Request::Wipe {
                confirm: Some(WIPE_CONFIRMATION.to_string()),
            },
            &state,
        )
        .await;
        drop(response);

        persist_last_fix_and_history(
            &state,
            stale_ticket,
            LastFix {
                lat: 55.0,
                lon: 12.0,
                accuracy_m: 5.0,
                address: None,
                at: chrono::Utc::now(),
                sources: 1,
            },
        )
        .await;
        assert!(state.last_fix.lock().await.is_none());
        assert!(lock_db(&state).get_last_fix().unwrap().is_none());
        assert!(lock_db(&state)
            .get_fixes_in_range("0000", "9999")
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn locate_with_ten_cached_aps_commits_touch_and_fix_batches_only() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        const OLD_LAST_SEEN: &str = "2000-01-01T00:00:00+00:00";
        let state = test_state();
        assert!(
            !state.args.address_approx,
            "test requires address lookup disabled"
        );
        assert_eq!(state.args.top_n, 10, "test requires all ten APs selected");
        let bssids: Vec<String> = (0..10)
            .map(|index| format!("02:00:00:00:10:{index:02x}"))
            .collect();
        for (index, bssid) in bssids.iter().enumerate() {
            cache_ap(
                &state,
                bssid,
                55.67 + index as f64 / 100_000.0,
                12.56 + index as f64 / 100_000.0,
            );
        }
        state
            .db
            .set_cached_last_seen_for_test(bssids.clone(), OLD_LAST_SEEN.to_owned())
            .await
            .unwrap();

        let sample: crate::debounce::ScanSample = bssids
            .iter()
            .enumerate()
            .map(|(index, bssid)| {
                (
                    bssid.clone(),
                    crate::debounce::ScanEntry {
                        signal_dbm: -30 - index as i32,
                        ssid: None,
                        channel: Some(1),
                    },
                )
            })
            .collect();
        {
            let mut debouncer = state.debouncer.lock().await;
            for _ in 0..state.args.debounce_threshold {
                debouncer.push_scan(sample.clone());
            }
        }

        let commits = Arc::new(AtomicUsize::new(0));
        state
            .db
            .install_writer_commit_counter(Arc::clone(&commits))
            .await
            .unwrap();
        commits.store(0, Ordering::SeqCst);
        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], true, "locate failed: {response}");
        assert_eq!(json["cached"], 10);
        assert_eq!(json["sources"], 10);
        for field in ["address", "stale", "age_s"] {
            assert!(
                json.get(field).is_none(),
                "fresh locate must omit {field:?} on the production wire path: {response}"
            );
        }

        state.db.writer_barrier().await.unwrap();
        assert_eq!(
            commits.load(Ordering::SeqCst),
            2,
            "production locate must commit one touch batch and one fix batch"
        );
        let last_seen = state.db.cached_last_seen_for_test(bssids).await.unwrap();
        assert!(last_seen.iter().all(|value| value != OLD_LAST_SEEN));
        let last_fix = state.db.get_last_fix().await.unwrap();
        assert!(last_fix.is_some(), "locate must persist last_fix");
        let history = state
            .db
            .get_fixes_in_range("2000-01-01T00:00:00+00:00", "2099-01-01T00:00:00+00:00")
            .await
            .unwrap();
        assert_eq!(history.len(), 1, "locate must append one history row");
    }

    #[tokio::test]
    async fn scan_and_debug_option_fields_pin_null_vs_omitted_wire_shape() {
        let state = test_state();

        let empty_scan = dispatch_command(&Request::Scan, &state).await;
        let empty_scan: serde_json::Value = serde_json::from_str(&empty_scan).unwrap();
        assert_eq!(
            empty_scan.get("scan_age_ms"),
            Some(&serde_json::Value::Null)
        );
        assert!(
            empty_scan.get("scanned_at").is_none(),
            "scanned_at is omitted until a scan exists: {empty_scan}"
        );

        let empty_debug = dispatch_command(&Request::Debug, &state).await;
        let empty_debug: serde_json::Value = serde_json::from_str(&empty_debug).unwrap();
        assert_eq!(
            empty_debug.get("scan_age_ms"),
            Some(&serde_json::Value::Null)
        );

        let bssid = "02:00:00:00:20:01";
        let mut sample = crate::debounce::ScanSample::new();
        sample.insert(
            bssid.to_string(),
            crate::debounce::ScanEntry {
                signal_dbm: -52,
                ssid: None,
                channel: None,
            },
        );
        {
            let mut debouncer = state.debouncer.lock().await;
            for _ in 0..state.args.debounce_threshold {
                debouncer.push_scan(sample.clone());
            }
            // Keep the BSSID stable in the ring while making it absent from the
            // latest scan, which is the production source of signal_dbm:null.
            debouncer.push_scan(crate::debounce::ScanSample::new());
        }

        let missing_signal = dispatch_command(&Request::Debug, &state).await;
        let missing_signal: serde_json::Value = serde_json::from_str(&missing_signal).unwrap();
        let row = missing_signal["bssids"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["bssid"] == bssid)
            .expect("stable BSSID must remain in debug output");
        assert_eq!(row.get("signal_dbm"), Some(&serde_json::Value::Null));

        state.debouncer.lock().await.push_scan(sample);
        let populated_scan = dispatch_command(&Request::Scan, &state).await;
        let populated_scan: serde_json::Value = serde_json::from_str(&populated_scan).unwrap();
        let network = &populated_scan["networks"][0];
        assert_eq!(network.get("ssid"), Some(&serde_json::Value::Null));
        assert_eq!(network.get("channel"), Some(&serde_json::Value::Null));
        assert!(populated_scan["scan_age_ms"].is_u64());
        assert!(populated_scan["scanned_at"].is_string());
    }

    /// task-0184: a history request whose range overflows chrono must return
    /// the protocol-mandated `ok:false` error envelope rather than panic the
    /// connection task (a panic would hand the client a dropped socket, not
    /// the documented error JSON).
    #[tokio::test]
    async fn history_huge_range_yields_ok_false_not_panic() {
        let state = test_state();
        let req = Request::History {
            range: Some("100000000d".to_string()),
            from: None,
            to: None,
        };
        // Returning normally at all is the "connection survives" property:
        // before task-0184 this call panicked inside parse_range.
        let resp = dispatch_command(&req, &state).await;
        let v: serde_json::Value =
            serde_json::from_str(&resp).expect("dispatch must return valid JSON");
        assert_eq!(
            v.get("ok").and_then(|b| b.as_bool()),
            Some(false),
            "huge range must yield ok:false, got {resp}"
        );
        assert!(
            v.get("error")
                .and_then(|e| e.as_str())
                .is_some_and(|s| s.contains("invalid range")),
            "error should explain the invalid range, got {resp}"
        );
    }

    fn one_hour_history_request() -> Request {
        Request::History {
            range: Some("1h".to_string()),
            from: None,
            to: None,
        }
    }

    #[tokio::test]
    async fn history_busy_is_prompt_and_response_owns_slot_until_drop() {
        let state = test_state();
        let held = Arc::clone(&state.history_slot)
            .try_acquire_owned()
            .expect("direct history gate");
        let busy = tokio::time::timeout(
            Duration::from_millis(250),
            dispatch_command(&one_hour_history_request(), &state),
        )
        .await
        .expect("busy history must not wait for the gate");
        assert!(busy.contains("history: busy; retry later"), "{busy}");
        drop(held);

        let first = dispatch_command(&one_hour_history_request(), &state).await;
        assert!(first.contains("\"ok\":true"), "{first}");
        assert_eq!(
            state.history_slot.available_permits(),
            0,
            "completed CPU work must not release admission before socket write"
        );
        let second = dispatch_command(&one_hour_history_request(), &state).await;
        assert!(second.contains("history: busy; retry later"), "{second}");
        drop(second);
        assert_eq!(state.history_slot.available_permits(), 0);

        drop(first);
        assert_eq!(
            state.history_slot.available_permits(),
            1,
            "dropping the response envelope models completed socket write"
        );
    }

    #[tokio::test]
    async fn history_admission_is_held_through_actual_stalled_socket_write() {
        let state = test_state();
        // One byte forces the real handle_connection write_all to stall while
        // its CommandResponse still owns history admission.
        let (client, server) = tokio::io::duplex(1);
        let handler_state = Arc::clone(&state);
        let handler = tokio::spawn(async move {
            handle_connection(server, handler_state, TransportKind::Unix).await
        });
        let (mut client_read, mut client_write) = tokio::io::split(client);
        client_write
            .write_all(b"{\"cmd\":\"history\",\"range\":\"1h\"}\n")
            .await
            .unwrap();
        client_write.shutdown().await.unwrap();

        let mut first_byte = [0_u8; 1];
        let first_read = tokio::time::timeout(
            Duration::from_secs(1),
            client_read.read_exact(&mut first_byte),
        )
        .await;
        let handler_stalled = !handler.is_finished();
        let held_during_write = Arc::clone(&state.history_slot).try_acquire_owned().is_err();
        let second = tokio::time::timeout(
            Duration::from_millis(250),
            dispatch_command(&one_hour_history_request(), &state),
        )
        .await;
        let second_body = second.map(|response| response.to_string());

        // Cleanup before assertions: drain the response so write_all and
        // writer.shutdown finish, then reap the real connection handler.
        let mut rest = Vec::new();
        let drain_result =
            tokio::time::timeout(Duration::from_secs(1), client_read.read_to_end(&mut rest)).await;
        drop(client_read);
        drop(client_write);
        let handler_result = tokio::time::timeout(Duration::from_secs(1), handler).await;
        let final_capacity = state.history_slot.available_permits();

        first_read
            .expect("history response did not begin")
            .expect("reading first history response byte failed");
        assert_eq!(first_byte[0], b'{');
        assert!(
            handler_stalled,
            "one-byte duplex must stall the handler in its response write"
        );
        assert!(
            held_during_write,
            "history admission must remain held while the socket write stalls"
        );
        let second_body = second_body.expect("second history did not fail promptly");
        assert!(
            second_body.contains("history: busy; retry later"),
            "second history response: {second_body}"
        );
        drain_result
            .expect("history response drain exceeded watchdog")
            .expect("history response drain failed");
        handler_result
            .expect("history connection handler did not finish")
            .expect("history connection task panicked")
            .expect("history connection handler failed");
        let mut wire = first_byte.to_vec();
        wire.extend(rest);
        let response: serde_json::Value =
            serde_json::from_slice(wire.strip_suffix(b"\n").unwrap_or(&wire)).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(final_capacity, 1);
    }

    /// The production blocking helper is gated after it starts on Tokio's
    /// blocking pool. On a one-worker runtime, a real locate and timer still
    /// complete while burst history calls fail fast instead of queueing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn blocking_history_does_not_starve_runtime_and_burst_is_busy() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let state = test_state();
        *state.last_fix.lock().await = Some(LastFix {
            lat: 55.6761,
            lon: 12.5683,
            accuracy_m: 20.0,
            address: None,
            at: chrono::Utc::now(),
            sources: 3,
        });
        let lease = Arc::new(
            Arc::clone(&state.history_slot)
                .try_acquire_owned()
                .expect("history lease"),
        );
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (watchdog_cancel_tx, watchdog_cancel_rx) = std::sync::mpsc::channel();
        let watchdog_fired = Arc::new(AtomicBool::new(false));
        let watchdog_flag = Arc::clone(&watchdog_fired);
        let watchdog_release = release_tx.clone();
        let watchdog = std::thread::spawn(move || {
            if watchdog_cancel_rx
                .recv_timeout(std::time::Duration::from_secs(3))
                .is_err()
            {
                watchdog_flag.store(true, Ordering::SeqCst);
                let _ = watchdog_release.send(());
            }
        });

        let (join, mut cancellation) = spawn_history_work(
            HistoryWork {
                fixes: Vec::new(),
                from: "2026-07-05T00:00:00+00:00".to_string(),
                to: "2026-07-05T01:00:00+00:00".to_string(),
                dist_threshold_m: 100.0,
                min_duration_secs: 300,
                shutdown: state.shutdown.clone(),
                lease: Arc::clone(&lease),
            },
            move |_| {
                let _ = started_tx.send(());
                release_rx.recv().expect("history gate release");
            },
        );
        tokio::time::timeout(Duration::from_secs(1), started_rx)
            .await
            .expect("history work did not enter the blocking pool")
            .expect("history start sender dropped");

        let responsiveness = tokio::time::timeout(Duration::from_millis(500), async {
            let locate = dispatch_command(&Request::Locate, &state);
            let timer = tokio::time::sleep(Duration::from_millis(20));
            let (locate, ()) = tokio::join!(locate, timer);
            locate
        })
        .await;
        let burst = tokio::time::timeout(Duration::from_millis(250), async {
            let mut responses = Vec::new();
            for _ in 0..8 {
                responses.push(dispatch_command(&one_hour_history_request(), &state).await);
            }
            responses
        })
        .await;

        // Cleanup precedes assertions so a regression cannot strand the
        // blocking pool during runtime teardown. The OS-thread watchdog is
        // only a last-resort release and must remain unused on the green path.
        let _ = release_tx.send(());
        let worker = tokio::time::timeout(Duration::from_secs(1), join).await;
        cancellation.disarm();
        drop(lease);
        let _ = watchdog_cancel_tx.send(());
        watchdog.join().unwrap();

        let locate = responsiveness.expect("locate/timer starved by history CPU work");
        let locate_json: serde_json::Value =
            serde_json::from_str(&locate).expect("locate must return JSON");
        assert_eq!(
            locate_json["ok"], true,
            "real locate must succeed while history CPU is blocked: {locate}"
        );
        assert_eq!(locate_json["lat"].as_f64(), Some(55.6761));
        let burst = burst.expect("busy history burst was not prompt");
        assert!(
            burst
                .iter()
                .all(|response| response.contains("history: busy; retry later")),
            "every concurrent history must be rejected"
        );
        let body = worker
            .expect("blocking history did not exit after release")
            .expect("blocking history panicked");
        assert!(body.contains("\"ok\":true"), "{body}");
        assert!(
            !watchdog_fired.load(Ordering::SeqCst),
            "external cleanup watchdog was needed"
        );
        assert_eq!(state.history_slot.available_permits(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn aborting_async_waiter_keeps_history_busy_until_blocking_exit() {
        let state = test_state();
        let lease = Arc::new(
            Arc::clone(&state.history_slot)
                .try_acquire_owned()
                .expect("history lease"),
        );
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (join, cancellation) = spawn_history_work(
            HistoryWork {
                fixes: Vec::new(),
                from: "2026-07-05T00:00:00+00:00".to_string(),
                to: "2026-07-05T01:00:00+00:00".to_string(),
                dist_threshold_m: 100.0,
                min_duration_secs: 300,
                shutdown: state.shutdown.clone(),
                lease: Arc::clone(&lease),
            },
            move |_| {
                let _ = started_tx.send(());
                release_rx.recv().expect("history gate release");
            },
        );
        drop(lease);
        let waiter = tokio::spawn(async move {
            let mut cancellation = cancellation;
            let result = join.await;
            cancellation.disarm();
            result
        });
        tokio::time::timeout(Duration::from_secs(1), started_rx)
            .await
            .expect("blocking closure did not start")
            .unwrap();

        waiter.abort();
        let waiter_result = waiter.await;
        let busy = tokio::time::timeout(
            Duration::from_millis(250),
            dispatch_command(&one_hour_history_request(), &state),
        )
        .await;

        // Release before assertions: even if cancellation/admission regresses,
        // the blocking closure cannot strand runtime teardown.
        release_tx.send(()).unwrap();
        let restored = tokio::time::timeout(
            Duration::from_secs(1),
            Arc::clone(&state.history_slot).acquire_owned(),
        )
        .await
        .expect("cancelled blocking closure did not exit")
        .expect("history semaphore closed");
        assert!(waiter_result.unwrap_err().is_cancelled());
        let busy = busy.expect("second history waited instead of failing busy");
        assert!(
            busy.contains("history: busy; retry later"),
            "detached closure must retain its lease: {busy}"
        );
        drop(busy);
        assert_eq!(
            state.history_slot.available_permits(),
            0,
            "acquiring here proves the detached closure released only on exit"
        );
        drop(restored);
        assert_eq!(state.history_slot.available_permits(), 1);
    }

    /// Drive the real history blocking helper and Drop cancellation guard
    /// through the production shutdown drain. `JoinHandle::abort` only drops
    /// the async waiter; the blocking closure retains history admission until
    /// it observes that real guard's predicate and exits.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_drain_cancels_real_history_work_and_restores_both_caps() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let connections = Arc::new(tokio::sync::Semaphore::new(1));
        let histories = Arc::new(tokio::sync::Semaphore::new(1));
        let connection = Arc::clone(&connections).try_acquire_owned().unwrap();
        let response_lease = Arc::new(
            Arc::clone(&histories)
                .try_acquire_owned()
                .expect("history admission"),
        );
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let cancellation_observed = Arc::new(AtomicBool::new(false));
        let blocking_observation = Arc::clone(&cancellation_observed);
        let (join, cancellation) = spawn_history_work(
            HistoryWork {
                fixes: Vec::new(),
                from: "2026-07-05T00:00:00+00:00".to_string(),
                to: "2026-07-05T01:00:00+00:00".to_string(),
                dist_threshold_m: 100.0,
                min_duration_secs: 300,
                // Keep daemon cancellation false: this test must observe the
                // async Drop guard's local cancellation specifically.
                shutdown: tokio_util::sync::CancellationToken::new(),
                lease: Arc::clone(&response_lease),
            },
            move |is_cancelled| {
                let _ = started_tx.send(());
                release_rx.recv().expect("history test gate release");
                blocking_observation.store(is_cancelled(), Ordering::SeqCst);
            },
        );

        let mut tracked = tokio::task::JoinSet::new();
        tracked.spawn(async move {
            let _connection = connection;
            let _response_lease = response_lease;
            let mut cancellation = cancellation;
            let result = join.await;
            cancellation.disarm();
            let _ = result;
        });
        tokio::time::timeout(Duration::from_secs(1), started_rx)
            .await
            .expect("real blocking history work did not start")
            .unwrap();

        let aborted =
            crate::run_shutdown_drain(Duration::from_millis(25), Vec::new(), None, tracked).await;
        let connection_capacity_after_drain = connections.available_permits();
        let history_held_before_release = Arc::clone(&histories).try_acquire_owned().is_err();

        // Cleanup before assertions. Acquiring the semaphore itself is the
        // happens-before proof that the closure observed cancellation, exited,
        // and dropped its final Arc<OwnedSemaphorePermit>.
        release_tx.send(()).unwrap();
        let restored = tokio::time::timeout(
            Duration::from_secs(1),
            Arc::clone(&histories).acquire_owned(),
        )
        .await
        .expect("real blocking history closure did not exit")
        .expect("history admission semaphore closed");
        let observed = cancellation_observed.load(Ordering::SeqCst);
        drop(restored);
        let final_history_capacity = histories.available_permits();

        assert_eq!(aborted, 1, "tracked history handler must be force-aborted");
        assert_eq!(
            connection_capacity_after_drain, 1,
            "reaped tracked handler must restore connection admission"
        );
        assert!(
            history_held_before_release,
            "detached blocking closure must retain history admission"
        );
        assert!(
            observed,
            "real HistoryCancellationGuard Drop must feed the blocking predicate"
        );
        assert_eq!(final_history_capacity, 1);
    }

    /// task-0194 AC#1: an uncached BSSID resolved while WiGLE is unconfigured
    /// (test_state uses empty creds -> wigle_lookup returns Skipped) must be
    /// reported as `unresolved`, NOT `not_found` — the daemon consulted no
    /// provider, so absence is not established. Same shape as quota-exhausted.
    #[tokio::test]
    async fn resolve_uncached_wigle_unconfigured_is_unresolved() {
        let state = test_state();
        let req = Request::Resolve {
            bssids: vec!["AA:BB:CC:DD:EE:01".to_string()],
        };
        let resp = dispatch_command(&req, &state).await;
        assert_eq!(
            resolve_source(&resp),
            "unresolved",
            "unconfigured WiGLE must yield unresolved, got {resp}"
        );
    }

    /// task-0194 AC#2: a resolve racing a background resolution of the same
    /// BSSID (another task holds the in-flight claim) must be reported as
    /// `unresolved`, NOT `not_found` — the owning task owns the verdict.
    #[tokio::test]
    async fn resolve_racing_inflight_claim_is_unresolved() {
        let state = test_state();
        let bssid = "AA:BB:CC:DD:EE:02".to_string();
        // Simulate a concurrent task already resolving this BSSID.
        state
            .inflight
            .lock()
            .unwrap()
            .insert(bssid.clone(), state.privacy.ticket());

        let req = Request::Resolve {
            bssids: vec![bssid],
        };
        let resp = dispatch_command(&req, &state).await;
        assert_eq!(
            resolve_source(&resp),
            "unresolved",
            "in-flight race must yield unresolved, got {resp}"
        );
    }

    /// task-0194 AC#3 (positive): `not_found` is still emitted when absence is
    /// authoritative — here the not_found table holds the BSSID within TTL, so
    /// the read-only resolve short-circuits and reports `not_found`.
    #[tokio::test]
    async fn resolve_table_not_found_is_reported_as_not_found() {
        let state = test_state();
        let bssid = "AA:BB:CC:DD:EE:03".to_string();
        crate::server::lock_db(&state)
            .insert_not_found(&bssid)
            .unwrap();

        let req = Request::Resolve {
            bssids: vec![bssid],
        };
        let resp = dispatch_command(&req, &state).await;
        assert_eq!(
            resolve_source(&resp),
            "not_found",
            "table-authoritative absence must be reported as not_found, got {resp}"
        );
    }

    /// task-0238 AC#2/#3: use real SQLite cache rows and full protocol
    /// dispatch. serde_json historically masked the defect by serializing
    /// non-finite f64 values as null, so `source=unresolved` is the decisive
    /// assertion: the poisoned row was rejected rather than served as cache.
    #[tokio::test]
    async fn resolve_non_finite_cached_position_is_unresolved() {
        let state = test_state();
        let poisoned = [
            ("AA:BB:CC:DD:EE:04", f64::INFINITY, 12.5),
            ("AA:BB:CC:DD:EE:05", 55.5, f64::NEG_INFINITY),
        ];

        for (bssid, lat, lon) in poisoned {
            cache_ap(&state, bssid, lat, lon);
            let req = Request::Resolve {
                bssids: vec![bssid.to_string()],
            };
            let resp = dispatch_command(&req, &state).await;
            let result = first_resolve_result(&resp);

            assert_eq!(
                result["source"], "unresolved",
                "poisoned cache row must be retryable, never cache/not_found: {resp}"
            );
            assert!(
                result["lat"].is_null() && result["lon"].is_null(),
                "poisoned cache row must emit null coordinates: {resp}"
            );
        }
    }

    /// Positive control for task-0238: the serve-boundary guard must preserve
    /// the established finite cache response and wire vocabulary.
    #[tokio::test]
    async fn resolve_finite_cached_position_remains_cache() {
        let state = test_state();
        let bssid = "AA:BB:CC:DD:EE:06";
        cache_ap(&state, bssid, 55.6684, 12.5541);

        let req = Request::Resolve {
            bssids: vec![bssid.to_string()],
        };
        let resp = dispatch_command(&req, &state).await;
        let result = first_resolve_result(&resp);

        assert_eq!(result["source"], "cache", "finite cache hit: {resp}");
        assert_eq!(result["lat"].as_f64(), Some(55.6684));
        assert_eq!(result["lon"].as_f64(), Some(12.5541));
        assert_eq!(result["ssid"].as_str(), Some("test-ap"));
    }

    /// task-0102: spawn_tracked runs the future and tracks it in the JoinSet
    /// so the shutdown drain can observe it.
    #[tokio::test]
    async fn spawn_tracked_runs_the_future() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let state = test_state();
        let ran = std::sync::Arc::new(AtomicBool::new(false));
        let ran2 = std::sync::Arc::clone(&ran);
        state.spawn_tracked(async move {
            ran2.store(true, Ordering::SeqCst);
        });
        // Drain the tracked set to force the task to completion.
        let mut set = std::mem::take(&mut *state.tasks.lock().unwrap());
        while set.join_next().await.is_some() {}
        assert!(
            ran.load(Ordering::SeqCst),
            "spawn_tracked must actually run the future"
        );
    }

    /// task-0190 AC#2 (structural): once shutdown is signalled, spawn_tracked
    /// refuses new work — the future is dropped unrun and nothing is added to
    /// the tracked JoinSet. This is the enforcement point for "no new resolver
    /// work after the shutdown signal".
    #[tokio::test]
    async fn spawn_tracked_refuses_after_cancel() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let state = test_state();
        state.shutdown.cancel();
        let ran = std::sync::Arc::new(AtomicBool::new(false));
        let ran2 = std::sync::Arc::clone(&ran);
        state.spawn_tracked(async move {
            ran2.store(true, Ordering::SeqCst);
        });
        // Give any erroneously-spawned task a chance to run.
        tokio::task::yield_now().await;
        assert!(
            !ran.load(Ordering::SeqCst),
            "future must not run after shutdown was signalled"
        );
        assert_eq!(
            state.tasks.lock().unwrap().len(),
            0,
            "nothing may be tracked after a refused spawn"
        );
    }

    #[test]
    fn spawn_racing_shutdown_cannot_escape_into_replacement_joinset() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct DropFlag(std::sync::Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let state = test_state();
        let connection_slots = Arc::new(tokio::sync::Semaphore::new(1));
        let connection_permit = Arc::clone(&connection_slots)
            .try_acquire_owned()
            .expect("test connection permit");
        let dropped = std::sync::Arc::new(AtomicBool::new(false));
        let captured = DropFlag(std::sync::Arc::clone(&dropped));
        let (prechecked_tx, prechecked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let spawn_state = Arc::clone(&state);
        let contender = std::thread::spawn(move || {
            spawn_state.spawn_tracked_after_precheck(
                async move {
                    let _connection_permit = connection_permit;
                    let _captured = captured;
                    std::future::pending::<()>().await;
                },
                || {
                    prechecked_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
            );
        });
        prechecked_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("contender must pass the cheap cancellation check");

        state.shutdown.cancel();
        let taken = std::mem::take(&mut *state.tasks.lock().unwrap());
        assert!(taken.is_empty(), "precondition: no task was owned yet");
        release_tx.send(()).unwrap();
        contender.join().unwrap();

        assert!(
            dropped.load(Ordering::SeqCst),
            "refused future must be destroyed"
        );
        assert_eq!(
            state.tasks.lock().unwrap().len(),
            0,
            "racing spawn must not land in the post-take replacement JoinSet"
        );
        assert_eq!(
            connection_slots.available_permits(),
            1,
            "refusing the raced handler must return its real admission permit"
        );
    }
}

#[cfg(test)]
mod privacy_response_tests {
    use super::*;

    #[tokio::test]
    async fn wipe_wins_before_response_write_and_replaces_stale_body() {
        let state = test_state();
        let ticket = state.privacy.ticket();
        let response = CommandResponse::privacy("pre-wipe private body".to_string(), ticket);
        let mut wipe = state.privacy.begin_wipe().await;
        wipe.advance_epoch();

        let prepare_state = Arc::clone(&state);
        let prepare = tokio::spawn(async move {
            let mut response = response;
            response.prepare_for_write(&prepare_state).await;
            response
        });
        tokio::task::yield_now().await;
        assert!(
            !prepare.is_finished(),
            "stale response must wait behind the wipe response boundary"
        );
        drop(wipe);
        let response = prepare.await.unwrap();
        assert_eq!(response.body, error_json(STALE_AFTER_WIPE_ERROR));
    }

    #[tokio::test]
    async fn wipe_response_holds_exclusive_gate_until_response_drop() {
        let state = test_state();
        let mut lease = state.privacy.begin_wipe().await;
        let current = lease.advance_epoch();
        let admission = Arc::new(WipeAdmission {
            _permit: Arc::clone(&state.wipe_slot).try_acquire_owned().unwrap(),
        });
        let response = CommandResponse::wipe("wipe success".to_string(), lease, admission);

        let read_state = Arc::clone(&state);
        let read = tokio::spawn(async move { read_state.privacy.read(current).await });
        tokio::task::yield_now().await;
        assert!(
            !read.is_finished(),
            "post-wipe publication must wait until the wipe response lifetime ends"
        );
        drop(response);
        assert!(read.await.unwrap().is_some());
    }
}

#[cfg(test)]
mod transport_tests {
    //! task-0196: unix-socket transport setup — self-bind + 0600, stale-socket
    //! recovery, non-socket refusal, socket-activation env parsing, and a full
    //! request/response round-trip over the unix socket.
    //!
    //! HERMETICITY: every socket lives under a unique per-test tempdir; nothing
    //! touches the default socket path, the real filesystem defaults, or a live
    //! daemon.
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn unique_tempdir(tag: &str) -> PathBuf {
        // Filesystem Unix sockets have a short fixed path limit. Keep the test
        // component compact even when nix-shell supplies a longer TMPDIR.
        let dir = std::env::temp_dir().join(format!(
            "wx-{tag}-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mode_bits(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// Exercise the real one-shot connection handler without binding a host
    /// socket. The returned string includes the protocol framing newline.
    async fn duplex_round_trip(request: &str) -> String {
        duplex_round_trip_with_state(request, test_state(), TransportKind::Unix).await
    }

    async fn duplex_round_trip_with_state(
        request: &str,
        state: Arc<DaemonState>,
        transport: TransportKind,
    ) -> String {
        let (client, server) = tokio::io::duplex(4096);
        let handler = tokio::spawn(handle_connection(server, state, transport));
        let (mut reader, mut writer) = tokio::io::split(client);
        writer.write_all(request.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.shutdown().await.unwrap();

        let mut response = String::new();
        reader.read_to_string(&mut response).await.unwrap();
        handler.await.unwrap().unwrap();
        response
    }

    #[tokio::test]
    async fn invalid_request_wire_errors_pin_unknown_and_missing_command_details() {
        let unknown = duplex_round_trip(r#"{"cmd":"bogus"}"#).await;
        assert_eq!(
            unknown,
            "{\"ok\":false,\"v\":1,\"error\":\"invalid request: unknown variant `bogus`, expected one of `locate`, `resolve`, `scan`, `stats`, `debug`, `version`, `history`, `wipe` at line 1 column 14\"}\n"
        );

        let missing = duplex_round_trip(r#"{"range":"7d"}"#).await;
        assert_eq!(
            missing,
            "{\"ok\":false,\"v\":1,\"error\":\"invalid request: missing field `cmd` at line 1 column 14\"}\n"
        );
    }

    #[tokio::test]
    async fn confirmed_wipe_succeeds_on_unix_wire_and_is_rejected_on_tcp_wire() {
        let unix_state = test_state();
        let ap = crate::db::ApInfo {
            bssid: "aa:bb:cc:dd:ee:01".to_string(),
            ssid: Some("private".to_string()),
            lat: 55.0,
            lon: 12.0,
            encryption: None,
            channel: None,
            frequency: None,
            city: None,
            country: None,
            source: crate::db::Source::Manual,
        };
        lock_db(&unix_state).upsert_ap(&ap).unwrap();
        let unix = duplex_round_trip_with_state(
            r#"{"cmd":"wipe","confirm":"delete-all-location-data"}"#,
            Arc::clone(&unix_state),
            TransportKind::Unix,
        )
        .await;
        let unix_json: serde_json::Value = serde_json::from_str(unix.trim()).unwrap();
        assert_eq!(unix_json["ok"], true);
        assert_eq!(unix_json["aps_deleted"], 1);
        assert_eq!(lock_db(&unix_state).cached_ap_count().unwrap(), 0);

        let tcp_state = test_state();
        lock_db(&tcp_state).upsert_ap(&ap).unwrap();
        let tcp = duplex_round_trip_with_state(
            r#"{"cmd":"wipe","confirm":"delete-all-location-data"}"#,
            Arc::clone(&tcp_state),
            TransportKind::Tcp,
        )
        .await;
        assert_eq!(tcp.trim(), error_json(WIPE_TCP_ERROR));
        assert_eq!(
            lock_db(&tcp_state).cached_ap_count().unwrap(),
            1,
            "TCP refusal must have no side effects"
        );
    }

    #[tokio::test]
    async fn confirmed_wipe_finishes_after_client_disconnects() {
        let state = test_state();
        let ap = crate::db::ApInfo {
            bssid: "aa:bb:cc:dd:ee:02".to_string(),
            ssid: Some("private".to_string()),
            lat: 55.0,
            lon: 12.0,
            encryption: None,
            channel: None,
            frequency: None,
            city: None,
            country: None,
            source: crate::db::Source::Manual,
        };
        lock_db(&state).upsert_ap(&ap).unwrap();
        *state.last_fix.lock().await = Some(LastFix {
            lat: 55.0,
            lon: 12.0,
            accuracy_m: 5.0,
            address: Some("private".to_string()),
            at: chrono::Utc::now(),
            sources: 1,
        });

        let (mut client, server) = tokio::io::duplex(256);
        let handler_state = Arc::clone(&state);
        let handler = tokio::spawn(async move {
            handle_connection(server, handler_state, TransportKind::Unix).await
        });
        client
            .write_all(b"{\"cmd\":\"wipe\",\"confirm\":\"delete-all-location-data\"}\n")
            .await
            .unwrap();
        drop(client);
        handler.await.unwrap().unwrap();

        assert_eq!(lock_db(&state).cached_ap_count().unwrap(), 0);
        assert!(state.last_fix.lock().await.is_none());
        assert_eq!(state.wipe_slot.available_permits(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accepted_wipe_survives_force_aborted_connection_handler() {
        let state = test_state();
        let ap = crate::db::ApInfo {
            bssid: "aa:bb:cc:dd:ee:03".to_string(),
            ssid: Some("private".to_string()),
            lat: 55.0,
            lon: 12.0,
            encryption: None,
            channel: None,
            frequency: None,
            city: None,
            country: None,
            source: crate::db::Source::Manual,
        };
        lock_db(&state).upsert_ap(&ap).unwrap();
        *state.last_fix.lock().await = Some(LastFix {
            lat: 55.0,
            lon: 12.0,
            accuracy_m: 5.0,
            address: Some("private".to_string()),
            at: chrono::Utc::now(),
            sources: 1,
        });

        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker_db = state.db.clone();
        let blocker =
            tokio::spawn(async move { blocker_db.hold_writer_until(entered_tx, release_rx).await });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        let (mut client, server) = tokio::io::duplex(256);
        let handler_state = Arc::clone(&state);
        let handler = tokio::spawn(async move {
            handle_connection(server, handler_state, TransportKind::Unix).await
        });
        client
            .write_all(b"{\"cmd\":\"wipe\",\"confirm\":\"delete-all-location-data\"}\n")
            .await
            .unwrap();
        loop {
            if state
                .privacy_wipes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .len()
                == 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }

        handler.abort();
        assert!(handler.await.unwrap_err().is_cancelled());
        drop(client);
        release_tx.send(()).unwrap();
        blocker.await.unwrap().unwrap();

        let mut wipes = {
            let mut guard = state
                .privacy_wipes
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            std::mem::take(&mut *guard)
        };
        while let Some(result) = wipes.join_next().await {
            result.unwrap();
        }

        assert_eq!(lock_db(&state).cached_ap_count().unwrap(), 0);
        assert!(state.last_fix.lock().await.is_none());
        assert_eq!(state.wipe_slot.available_permits(), 1);
    }

    #[tokio::test]
    async fn wipe_holds_exclusive_gate_through_stalled_socket_write() {
        let state = test_state();
        let before = state.privacy.ticket();
        *state.last_fix.lock().await = Some(LastFix {
            lat: 55.0,
            lon: 12.0,
            accuracy_m: 5.0,
            address: Some("private".to_string()),
            at: chrono::Utc::now(),
            sources: 1,
        });

        // One byte of capacity makes the success response block until this
        // test starts reading it. That exposes the actual socket-write
        // lifetime rather than merely inspecting CommandResponse ownership.
        let (client, server) = tokio::io::duplex(1);
        let handler_state = Arc::clone(&state);
        let handler = tokio::spawn(async move {
            handle_connection(server, handler_state, TransportKind::Unix).await
        });
        let (client_read, mut client_write) = tokio::io::split(client);
        client_write
            .write_all(b"{\"cmd\":\"wipe\",\"confirm\":\"delete-all-location-data\"}\n")
            .await
            .unwrap();

        // Wait for the production DB -> managed-memory clear -> final epoch
        // advance sequence. The worker then retains the write lease or has
        // transferred it to the response stalled on the one-byte socket.
        while state.privacy.ticket() == before {
            tokio::task::yield_now().await;
        }
        assert!(state.last_fix.lock().await.is_none());
        let current = state.privacy.ticket();
        let read_state = Arc::clone(&state);
        let publication = tokio::spawn(async move { read_state.privacy.read(current).await });
        tokio::task::yield_now().await;
        assert!(
            !publication.is_finished(),
            "post-wipe publication escaped before the response-write attempt ended"
        );
        let overlapping = tokio::time::timeout(
            Duration::from_millis(100),
            dispatch_command(
                &Request::Wipe {
                    confirm: Some(WIPE_CONFIRMATION.to_string()),
                },
                &state,
            ),
        )
        .await
        .expect("stalled success write must reject an overlapping wipe promptly");
        assert_eq!(overlapping.body, error_json(WIPE_BUSY_ERROR));
        assert_eq!(state.wipe_slot.available_permits(), 0);

        let mut line = String::new();
        BufReader::new(client_read)
            .read_line(&mut line)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(line.trim()).unwrap()["ok"],
            true
        );
        handler.await.unwrap().unwrap();
        assert!(publication.await.unwrap().is_some());
        assert_eq!(state.wipe_slot.available_permits(), 1);
    }

    #[test]
    fn self_bind_creates_0600_socket() {
        let dir = unique_tempdir("mode");
        let sock = dir.join("whereamid.sock");
        let listener = self_bind_unix(&sock).expect("self-bind must succeed on a clean path");
        assert_eq!(
            mode_bits(&sock),
            0o600,
            "self-bound control socket must be owner-only (0600)"
        );
        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn self_bind_recovers_from_stale_socket() {
        let dir = unique_tempdir("stale");
        let sock = dir.join("whereamid.sock");
        // Create a stale socket inode: bind a std listener, then drop it. std
        // does NOT unlink on drop, so the socket file remains with no listener —
        // exactly the leftover a daemon that was SIGKILLed leaves behind.
        {
            let dead = std::os::unix::net::UnixListener::bind(&sock).unwrap();
            drop(dead);
        }
        assert!(
            sock.exists(),
            "stale socket inode must persist after the listener is dropped"
        );
        // Must detect the stale inode (connect refused), unlink, bind, chmod.
        let listener = self_bind_unix(&sock).expect("must recover from a stale socket file");
        assert_eq!(mode_bits(&sock), 0o600);
        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn self_bind_refuses_live_socket() {
        // Holding a LIVE (bound, listening) socket makes the connect probe
        // succeed even without an accept() — the kernel completes the handshake
        // up to the backlog — so self-bind must refuse rather than steal it.
        let dir = unique_tempdir("live");
        let sock = dir.join("whereamid.sock");
        let live = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let err = self_bind_unix(&sock).expect_err("must refuse to steal a live socket");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("already listening") || msg.contains("live control socket"),
            "error must explain the live-socket refusal, got: {msg}"
        );
        drop(live);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn self_bind_refuses_non_socket_file() {
        let dir = unique_tempdir("notsock");
        let path = dir.join("whereamid.sock");
        std::fs::write(&path, b"not a socket").unwrap();
        let err = self_bind_unix(&path).expect_err("must refuse to clobber a non-socket file");
        assert!(
            format!("{err:#}").contains("not a socket"),
            "error must explain the non-socket refusal"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_listener_fd_accepts_listening_unix_socket() {
        let dir = unique_tempdir("verify-ok");
        let sock = dir.join("whereamid.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        verify_unix_listener_fd(&listener).expect("a bound+listening unix socket must pass");
        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_listener_fd_rejects_connected_socket() {
        // A CONNECTED unix socket (socketpair-style) is AF_UNIX but not
        // listening: SO_ACCEPTCONN must catch it. This is the "systemd handed
        // us a Connect= / wrong fd" case.
        use std::os::unix::io::{AsRawFd, FromRawFd};
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        // Reborrow the stream fd as a listener WITHOUT taking ownership:
        // ManuallyDrop never runs the listener's close, so `a` closes the fd
        // exactly once even if an assertion below panics.
        let fd = a.as_raw_fd();
        // Safety: the fake listener is ManuallyDrop, so ownership stays with
        // `a`; the fd outlives every use of `fake` in this scope.
        let fake = std::mem::ManuallyDrop::new(unsafe {
            std::os::unix::net::UnixListener::from_raw_fd(fd)
        });
        let err = verify_unix_listener_fd(&fake).expect_err("connected socket must be rejected");
        assert!(
            format!("{err:#}").contains("not listening"),
            "error must say the fd is not listening, got: {err:#}"
        );
    }

    #[test]
    fn verify_listener_fd_rejects_regular_file() {
        // A non-socket fd (regular file) must fail cleanly at getsockname
        // instead of surfacing later as a confusing accept/epoll error.
        use std::os::unix::io::{FromRawFd, IntoRawFd};
        let dir = unique_tempdir("verify-file");
        let path = dir.join("plain-file");
        std::fs::write(&path, b"x").unwrap();
        let fd = std::fs::File::open(&path).unwrap().into_raw_fd();
        // Safety: we own fd (into_raw_fd released it); the fake listener now
        // owns it and closes it exactly once on drop.
        let fake = unsafe { std::os::unix::net::UnixListener::from_raw_fd(fd) };
        let err = verify_unix_listener_fd(&fake).expect_err("regular file must be rejected");
        assert!(
            format!("{err:#}").contains("not an AF_UNIX socket"),
            "error must say the fd is not a unix socket, got: {err:#}"
        );
        drop(fake);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_cloexec_sets_the_flag() {
        use std::os::unix::io::AsRawFd;
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        let fd = a.as_raw_fd();
        // Rust creates sockets with SOCK_CLOEXEC already; clear it first so the
        // test proves set_cloexec SETS the flag rather than observing a default.
        // Safety: plain fcntl on an fd we own; no memory passed.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            assert!(flags >= 0);
            assert_eq!(libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC), 0);
        }
        set_cloexec(fd).expect("set_cloexec must succeed on an open fd");
        // Safety: as above.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert!(
            flags >= 0 && (flags & libc::FD_CLOEXEC) != 0,
            "FD_CLOEXEC must be set after set_cloexec (flags={flags})"
        );
    }

    #[test]
    fn parse_listen_fds_cases() {
        // Not socket-activated (env absent).
        assert_eq!(parse_listen_fds(None, None, 42).unwrap(), None);
        // Addressed to a different pid → ignored (fall through to self-bind).
        assert_eq!(parse_listen_fds(Some("999"), Some("1"), 42).unwrap(), None);
        // Exactly one fd for us → fd 3.
        assert_eq!(
            parse_listen_fds(Some("42"), Some("1"), 42).unwrap(),
            Some(SD_LISTEN_FDS_START)
        );
        // More than one fd is a hard error (whereamid uses a single socket).
        assert!(parse_listen_fds(Some("42"), Some("2"), 42).is_err());
        // Garbage values fail fast.
        assert!(parse_listen_fds(Some("notanum"), Some("1"), 42).is_err());
        assert!(parse_listen_fds(Some("42"), Some("x"), 42).is_err());
    }

    /// Full round-trip: run_server accepts on a self-bound unix socket and
    /// answers a `version` request. Uses a std blocking client on a blocking
    /// thread (faithful to the real client), so a multi-thread runtime is
    /// required. Also re-asserts the 0600 mode on the live socket.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unix_round_trip_version() {
        let dir = unique_tempdir("roundtrip");
        let sock = dir.join("whereamid.sock");
        let std_listener = self_bind_unix(&sock).unwrap();
        assert_eq!(mode_bits(&sock), 0o600, "live socket must be 0600");
        std_listener.set_nonblocking(true).unwrap();
        let listener = UnixListener::from_std(std_listener).unwrap();

        // test_state's args have bind=None, so run_server opens no TCP listener.
        let state = test_state();
        let server = tokio::spawn(run_server(state, listener));

        let sock2 = sock.clone();
        let line = tokio::task::spawn_blocking(move || {
            use std::io::{BufRead, BufReader, Write};
            let mut s = std::os::unix::net::UnixStream::connect(&sock2).expect("connect");
            s.write_all(b"{\"cmd\":\"version\"}\n").unwrap();
            s.flush().unwrap();
            s.shutdown(std::net::Shutdown::Write).unwrap();
            let mut line = String::new();
            BufReader::new(s).read_line(&mut line).unwrap();
            line
        })
        .await
        .unwrap();

        server.abort();
        let _ = std::fs::remove_dir_all(&dir);

        let v: serde_json::Value =
            serde_json::from_str(line.trim()).expect("response must be one JSON line");
        assert_eq!(v["ok"], serde_json::json!(true), "got {line}");
        assert_eq!(v["v"], serde_json::json!(1));
        assert!(
            v.get("version").and_then(|x| x.as_str()).is_some(),
            "version response must carry a version string, got {line}"
        );
    }

    /// Behavioral parity for the opt-in TCP transport: the same generic
    /// `handle_connection` answers a `version` request over a TcpStream
    /// (QA review LOW-3 — before this, TCP was exercised only via bind
    /// success/failure, never a real exchange). Binds an ephemeral kernel-
    /// assigned port on loopback; hermetic, never port 4747.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tcp_round_trip_version_via_handle_connection() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert_ne!(addr.port(), 4747, "must never test against the real port");

        let state = test_state();
        let server = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.unwrap();
            handle_connection(stream, state, TransportKind::Tcp)
                .await
                .unwrap();
        });

        let line = tokio::task::spawn_blocking(move || {
            use std::io::{BufRead, BufReader, Write};
            let mut s = std::net::TcpStream::connect(addr).expect("connect");
            s.write_all(b"{\"cmd\":\"version\"}\n").unwrap();
            s.flush().unwrap();
            s.shutdown(std::net::Shutdown::Write).unwrap();
            let mut line = String::new();
            BufReader::new(s).read_line(&mut line).unwrap();
            line
        })
        .await
        .unwrap();

        server.await.unwrap();

        let v: serde_json::Value =
            serde_json::from_str(line.trim()).expect("response must be one JSON line");
        assert_eq!(v["ok"], serde_json::json!(true), "got {line}");
        assert_eq!(v["v"], serde_json::json!(1));
    }

    /// task-0201: admission happens before accept. Two silent clients consume
    /// exactly two tracked handlers; a third remains in the kernel backlog and
    /// is only accepted after one handler returns its permit.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shared_connection_cap_keeps_excess_in_backlog_and_restores_capacity() {
        async fn wait_for_tracked(state: &Arc<DaemonState>, expected: usize) {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if state.tasks.lock().unwrap().len() == expected {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("tracked handler count never reached {expected}"));
        }

        let dir = unique_tempdir("connection-cap");
        let sock = dir.join("whereamid.sock");
        let std_listener = self_bind_unix(&sock).unwrap();
        std_listener.set_nonblocking(true).unwrap();
        let listener = UnixListener::from_std(std_listener).unwrap();
        let state = test_state();
        let slots = Arc::new(tokio::sync::Semaphore::new(2));
        let server_state = Arc::clone(&state);
        let server_slots = Arc::clone(&slots);
        let server = tokio::spawn(async move {
            run_server_inner(server_state, listener, None, server_slots).await
        });

        let first = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let second = tokio::net::UnixStream::connect(&sock).await.unwrap();
        wait_for_tracked(&state, 2).await;
        assert_eq!(slots.available_permits(), 0);

        let mut third = tokio::time::timeout(
            Duration::from_secs(2),
            tokio::net::UnixStream::connect(&sock),
        )
        .await
        .expect("third connect exceeded backlog watchdog")
        .unwrap();
        third.write_all(b"{\"cmd\":\"version\"}\n").await.unwrap();
        assert_eq!(
            state.tasks.lock().unwrap().len(),
            2,
            "third connection must not spawn a permit-waiting task"
        );

        drop(first);
        let mut line = String::new();
        tokio::time::timeout(
            Duration::from_secs(2),
            BufReader::new(&mut third).read_line(&mut line),
        )
        .await
        .expect("third client was not admitted after capacity returned")
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["ok"], true, "third client response: {line}");

        // The idle accept loop intentionally pre-acquires one permit. Abort it
        // before asserting full restoration, then reap every tracked handler.
        server.abort();
        let _ = server.await;
        drop(second);
        drop(third);
        let mut tracked = std::mem::take(&mut *state.tasks.lock().unwrap());
        tokio::time::timeout(Duration::from_secs(2), async {
            while tracked.join_next().await.is_some() {}
        })
        .await
        .expect("connection handlers did not drain");
        assert_eq!(
            slots.available_permits(),
            2,
            "accept abort and handler completion must restore every permit"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unix_and_tcp_share_one_connection_cap_and_wire_framing() {
        let dir = unique_tempdir("shared-transport-cap");
        let sock = dir.join("whereamid.sock");
        let std_unix = self_bind_unix(&sock).unwrap();
        std_unix.set_nonblocking(true).unwrap();
        let unix = UnixListener::from_std(std_unix).unwrap();
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_addr = tcp.local_addr().unwrap();
        let state = test_state();
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let server_state = Arc::clone(&state);
        let server_slots = Arc::clone(&slots);
        let server = tokio::spawn(async move {
            run_server_inner(server_state, unix, Some(tcp), server_slots).await
        });

        let unix_client = tokio::net::UnixStream::connect(&sock).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while state.tasks.lock().unwrap().len() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("unix handler was not admitted");
        assert_eq!(slots.available_permits(), 0);

        let mut tcp_client = tokio::net::TcpStream::connect(tcp_addr).await.unwrap();
        tcp_client
            .write_all(b"{\"cmd\":\"version\"}\n")
            .await
            .unwrap();
        assert_eq!(
            state.tasks.lock().unwrap().len(),
            1,
            "TCP must not bypass the unix-owned shared cap"
        );
        drop(unix_client);

        let mut line = String::new();
        tokio::time::timeout(
            Duration::from_secs(2),
            BufReader::new(&mut tcp_client).read_line(&mut line),
        )
        .await
        .expect("TCP backlog client was not admitted")
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(json["ok"], true, "shared generic handler response: {line}");

        server.abort();
        let _ = server.await;
        drop(tcp_client);
        let mut tracked = std::mem::take(&mut *state.tasks.lock().unwrap());
        tokio::time::timeout(Duration::from_secs(2), async {
            while tracked.join_next().await.is_some() {}
        })
        .await
        .expect("shared unix/TCP handlers did not drain");
        assert_eq!(slots.available_permits(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
