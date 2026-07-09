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
use crate::boot_clock::BootTime;
use crate::config::Args;
use crate::db::{DbHandle, DebugDbStatus, LastFix};
use crate::debounce::{Debouncer, TimestampedScan};
use crate::nominatim::NominatimClient;
use crate::wigle::WigleClient;
// The JSON wire types are defined once in `whereami-protocol` (task-0114); the
// daemon serializes them and the client deserializes the identical types. The
// daemon is strict by construction: it never builds the tolerant `Unknown`
// variant of `Provenance`/`DbStatus`, so its emitted JSON is unchanged.
use whereami_protocol::{
    DbStatus, DebugBssid, DebugResponse, ErrorResponse, HistoryFix, HistoryFixesResponse,
    HistoryRange, HistoryResponse, LocateResponse, NetworkEntry, Provenance, ResolveResponse,
    ResolveResultEntry, ScanResponse, StatsResponse, VersionResponse, WipeResponse,
    MAX_RESOLVE_BSSIDS,
};

const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Total budget to write the response and close the socket. A slow client
/// reading slowly should not pin a tokio task with the response buffered
/// indefinitely. Symmetric to READ_TIMEOUT in magnitude (task-0080).
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST_BYTES: u64 = 64 * 1024; // 64 KiB max request size

// MAX_RESOLVE_BSSIDS now lives in whereami-protocol as the single source of
// truth shared with the client's fail-fast guard (task-0215); imported above.
const DEFAULT_RAW_HISTORY_MAX_FIXES: u64 = 1_000;
const MAX_RAW_HISTORY_FIXES: u64 = 10_000;

/// Wire protocol version emitted in every daemon response. Breaking
/// wire-format changes bump this; clients use it to detect the daemon's
/// protocol version.
const WIRE_VERSION: u32 = 1;
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

    /// Bump the cumulative counter of SWALLOWED DB read failures — reads that
    /// returned `Err` and were handled by continuing with a default/empty
    /// result, so the fault is otherwise invisible to ops (unlike a read error
    /// that already surfaces an error response, e.g. `get_last_fix` on the
    /// locate path, which is NOT counted here because it is already visible).
    /// Surfaced via the stats endpoint so a real DB fault (corruption, disk
    /// full, lock contention) does not masquerade as "no data" — a miss that
    /// wastes API quota, a zeroed stat, or a silently aborted drain.
    /// task-0105 (cache-lookup entry points), task-0252 (resolve_chain
    /// skip-cached / skip-not_found, pending drain, stats + debug reporting).
    /// Call it at every such warn-and-continue-with-default DB-read site
    /// REACHABLE after `DaemonState` exists. (Startup rehydration reads in
    /// `main.rs` — e.g. the initial `get_last_fix` / `get_metadata` — predate
    /// the counter's construction and so cannot bump it; they warn only.)
    pub fn record_db_read_failure(&self) {
        self.db_read_failures
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
    /// Cumulative count of best-effort DB reads that failed and were
    /// warned-only (cache lookups). Surfaced via the stats endpoint
    /// (task-0105) so silent read errors — which waste API quota by looking
    /// like cache misses and triggering redundant background resolves — are
    /// visible without scraping the journal. Reset on daemon restart.
    pub db_read_failures: Arc<std::sync::atomic::AtomicU64>,
    /// Successfully committed autonomous history rows since daemon start.
    /// Manual locate writes are deliberately excluded. Reset on restart.
    pub autolocate_fixes_written: Arc<std::sync::atomic::AtomicU64>,
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

// task-0134: the cached last-known position (`LastFix`) and the Row<->domain
// marshalling now live in `crate::db` (the DB layer owns RFC3339<->DateTime and
// the task-0238 non-finite guard). `LastFix` is imported at the top of this
// module. The former open-coded `From`/`TryFrom` adapters are gone; callers pass
// domain `LastFix` values straight to the DB methods.

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
    P: FnOnce(DbHandle, LastFix) -> F + Send,
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
    let result = persist(state.db.clone(), fix.clone()).await;
    // task-0046: prove the guard spans the persistence call. Do not move this
    // drop above `persist`; the concurrency regression asserts this ordering.
    drop(last);
    result?;
    Ok(true)
}

/// Publish a current winner without address lookup. The caller owns the
/// privacy read guard and `last_fix` mutex across this complete operation.
/// Keeping this as the single locked implementation lets policy dispatch and
/// the concurrency regressions exercise identical memory/SQLite semantics.
async fn persist_last_fix_and_history_locked(
    state: &Arc<DaemonState>,
    last: &mut Option<LastFix>,
    new_fix: LastFix,
    quality: FixQuality,
) {
    let persisted = new_fix.clone();
    *last = Some(new_fix);

    if let Err(e) = state.db.persist_fix_and_history(persisted, quality).await {
        warn!("failed to persist last_fix and history: {e}");
        state.record_db_failure();
    }
}

/// Publish a current winner with the existing address-cache/geocode behavior.
/// The caller owns the privacy read guard and `last_fix` mutex. The address
/// cache remains synchronous and is released before the durable writer await.
async fn persist_locate_fix_and_schedule_locked(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
    last: &mut Option<LastFix>,
    mut new_fix: LastFix,
    quality: FixQuality,
) -> Option<String> {
    let (address, row) = {
        let cache = state.address_cache.lock().unwrap_or_else(|error| {
            warn!("address cache mutex poisoned — recovering");
            error.into_inner()
        });
        let address = cache.get(new_fix.lat, new_fix.lon);
        new_fix.address = address.clone();
        let persisted = new_fix.clone();
        state
            .geocode
            .observe(new_fix.lat, new_fix.lon, address.is_none(), ticket);
        *last = Some(new_fix);
        (address, persisted)
    };

    if let Err(error) = state.db.persist_fix_and_history(row, quality).await {
        warn!("failed to persist last_fix and history: {error}");
        state.record_db_failure();
    }
    address
}

/// Test seam for the privacy/last_fix acquisition boundary. Persistence is
/// delegated to the production locked implementation above.
#[cfg(test)]
async fn persist_last_fix_and_history(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
    new_fix: LastFix,
) {
    let Some(_privacy) = state.privacy.read(ticket).await else {
        return;
    };
    let mut last = state.last_fix.lock().await;
    persist_last_fix_and_history_locked(
        state,
        &mut last,
        new_fix,
        FixQuality::new(None, None, None, Confidence::Low, None).expect("test quality is valid"),
    )
    .await;
}

/// Test seam used by geocode scheduler concurrency tests. All publication
/// behavior lives in the production locked helper above.
#[cfg(test)]
async fn persist_locate_fix_and_schedule(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
    new_fix: LastFix,
) -> Option<String> {
    let _privacy = state.privacy.read(ticket).await?;
    let mut last = state.last_fix.lock().await;
    persist_locate_fix_and_schedule_locked(
        state,
        ticket,
        &mut last,
        new_fix,
        FixQuality::new(None, None, None, Confidence::Low, None).expect("test quality is valid"),
    )
    .await
}

// task-0134: the task-0238 non-finite/invalid-accuracy rehydration tests moved
// to `crate::db` alongside the relocated guard (`last_fix_from_columns`), since
// NaN/±Inf cannot round-trip through a SQLite REAL column and so must be tested
// against the conversion function directly rather than via `get_last_fix`.

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
        /// Return raw fixes instead of stay-point segments.
        #[serde(default)]
        raw: bool,
        /// Optional raw-fix cap. Invalid unless `raw` is true.
        #[serde(default)]
        max_fixes: Option<u64>,
    },
    Wipe {
        #[serde(default)]
        confirm: Option<String>,
    },
}

use crate::quality::{Confidence, FixQuality};

fn error_json(msg: &str) -> String {
    serde_json::to_string(&ErrorResponse {
        ok: false,
        v: WIRE_VERSION,
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
        Request::History {
            range,
            from,
            to,
            raw,
            max_fixes,
        } => {
            handle_history(
                range.as_deref(),
                from.as_deref(),
                to.as_deref(),
                *raw,
                *max_fixes,
                state,
                ticket,
            )
            .await
        }
        Request::Wipe { confirm } => handle_wipe(confirm.as_deref(), state, transport).await,
    }
}

/// One scanner-eligible AP observed in the latest scan, carrying the fields the
/// locate pipeline threads from the debouncer boundary to trilateration input:
/// its BSSID, measured RSSI, and SSID (used for mobile-AP classification and
/// provider backfill). Replaces the former parallel `(String, i32, Option<String>)`
/// tuples so field access is named rather than positional.
#[derive(Debug, Clone)]
struct Candidate {
    bssid: String,
    signal_dbm: i32,
    /// Provenance of `signal_dbm`, carried from the debounce sample so it
    /// reaches `PositionedAp`/trilateration (task-0126). `signal_dbm` is still
    /// the value used on the wire and in DB.
    rssi_source: crate::rssi::RssiSource,
    ssid: Option<String>,
}

/// Select bounded cold-start provider work after cache lookup. Cached rows are
/// removed before the limit because the provider chain skips them; allowing
/// persisted-mobile or non-finite cache hits to occupy this list would waste
/// every slot and starve genuinely unknown scanner-eligible APs.
fn bounded_uncached_observations(
    candidates: &[Candidate],
    cached_bssids: &std::collections::HashSet<String>,
    limit: usize,
) -> Vec<crate::resolver::ObservedAp> {
    candidates
        .iter()
        .filter(|candidate| !cached_bssids.contains(&candidate.bssid))
        .take(limit)
        .map(|candidate| crate::resolver::ObservedAp {
            bssid: candidate.bssid.clone(),
            ssid: candidate.ssid.clone(),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlausibilityWinner {
    CurrentTrilat,
    LastFix,
}

#[derive(Debug)]
struct PlausibilityDecision {
    winner: PlausibilityWinner,
    /// The single validated bundle used by both the response and, for a
    /// current winner, the history INSERT. It is never recomputed downstream.
    quality: FixQuality,
    distance_m: f64,
    current_score: Option<f64>,
    last_fix_score: Option<f64>,
}

fn confidence_from_score(score: Option<f64>) -> Confidence {
    match score {
        Some(value) if value >= 0.7 => Confidence::High,
        Some(value) if value >= 0.4 => Confidence::Medium,
        _ => Confidence::Low,
    }
}

fn signed_elapsed_seconds(
    now: chrono::DateTime<chrono::Utc>,
    previous: chrono::DateTime<chrono::Utc>,
) -> f64 {
    (now - previous).as_seconds_f64()
}

fn format_score(score: Option<f64>) -> String {
    score
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn evaluate_plausibility(
    previous: Option<&LastFix>,
    current: (f64, f64),
    visible_cached: &[(f64, f64)],
    now: chrono::DateTime<chrono::Utc>,
    max_travel_speed_kmh: f64,
) -> PlausibilityDecision {
    // INVARIANT for every FixQuality::new(...).expect(...) below: the bundle is
    // always valid, so the expect cannot fire. ap_consensus_score returns None
    // or inliers/len with len >= MIN_CONSENSUS_N (finite, in 0..=1); age_secs is
    // clamped >= 0; and haversine_kmh clamps a non-positive/backward elapsed to
    // 0.0 speed, so speed is finite and >= 0. If any of those guards is ever
    // relaxed, an out-of-domain score/speed would abort the daemon here rather
    // than merely mis-report a response — keep them intact.
    let current_score = crate::trilaterate::ap_consensus_score(visible_cached, current);
    let Some(previous) = previous else {
        return PlausibilityDecision {
            winner: PlausibilityWinner::CurrentTrilat,
            quality: FixQuality::new(
                None,
                None,
                current_score,
                confidence_from_score(current_score),
                None,
            )
            .expect("plausibility produces a valid cold-start quality bundle"),
            distance_m: 0.0,
            current_score,
            last_fix_score: None,
        };
    };

    let elapsed_seconds = signed_elapsed_seconds(now, previous.at);
    if elapsed_seconds < 0.0 {
        warn!(
            previous_at = %previous.at,
            now = %now,
            "last_fix timestamp is in the future; clamping reported age and implied speed to zero"
        );
    }
    let age_secs = (now - previous.at).num_seconds().max(0);
    let distance_m = crate::geo::haversine_m(previous.lat, previous.lon, current.0, current.1);
    let speed = crate::geo::haversine_kmh(
        previous.lat,
        previous.lon,
        current.0,
        current.1,
        elapsed_seconds,
    );

    if speed <= max_travel_speed_kmh {
        return PlausibilityDecision {
            winner: PlausibilityWinner::CurrentTrilat,
            quality: FixQuality::new(
                Some(speed),
                Some(age_secs),
                current_score,
                confidence_from_score(current_score),
                None,
            )
            .expect("plausibility produces a valid normal quality bundle"),
            distance_m,
            current_score,
            last_fix_score: None,
        };
    }

    let previous_score =
        crate::trilaterate::ap_consensus_score(visible_cached, (previous.lat, previous.lon));
    // Both scores are defined over the same denominator, so they are either
    // both Some or both None. Any tie (including undefined) deliberately keeps
    // current_trilat, the daemon's best current estimate.
    let winner = match (previous_score, current_score) {
        (Some(previous), Some(current)) if previous > current => PlausibilityWinner::LastFix,
        _ => PlausibilityWinner::CurrentTrilat,
    };
    let (winner_name, winner_score, loser_name, loser_score) = match winner {
        PlausibilityWinner::CurrentTrilat => {
            ("current_trilat", current_score, "last_fix", previous_score)
        }
        PlausibilityWinner::LastFix => {
            ("last_fix", previous_score, "current_trilat", current_score)
        }
    };
    let suspect_reason = format!(
        "implied {speed:.1} km/h exceeds max {max_travel_speed_kmh}; tie-break favored \
         {winner_name} (score {winner_name} {} vs {loser_name} {})",
        format_score(winner_score),
        format_score(loser_score),
    );

    PlausibilityDecision {
        winner,
        quality: FixQuality::new(
            Some(speed),
            Some(age_secs),
            winner_score,
            Confidence::Suspect,
            Some(suspect_reason),
        )
        .expect("plausibility produces a valid suspect quality bundle"),
        distance_m,
        current_score,
        last_fix_score: previous_score,
    }
}

fn autolocate_is_duplicate(
    previous: &LastFix,
    current: (f64, f64),
    now: chrono::DateTime<chrono::Utc>,
    distance_threshold_m: u64,
    interval_secs: u64,
) -> bool {
    let signed_age = now.signed_duration_since(previous.at);
    let age = if signed_age < chrono::TimeDelta::zero() {
        warn!(
            previous_at = %previous.at,
            now = %now,
            "autolocate last_fix timestamp is in the future; using zero age for dedup"
        );
        chrono::TimeDelta::zero()
    } else {
        signed_age
    };

    let distance_m = crate::geo::haversine_m(previous.lat, previous.lon, current.0, current.1);
    autolocate_metrics_are_duplicate(distance_m, age, distance_threshold_m, interval_secs)
}

fn autolocate_metrics_are_duplicate(
    distance_m: f64,
    age: chrono::TimeDelta,
    distance_threshold_m: u64,
    interval_secs: u64,
) -> bool {
    // DateTime's representable span is much smaller than u64::MAX seconds.
    // An interval outside TimeDelta's range therefore necessarily exceeds
    // every nonnegative age. This avoids lossy integer/f64 boundary checks.
    let age_inside_interval = i64::try_from(interval_secs)
        .ok()
        .and_then(chrono::TimeDelta::try_seconds)
        .is_none_or(|interval| age < interval);
    distance_m < distance_threshold_m as f64 && age_inside_interval
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocatePurpose {
    Interactive,
    Autolocate,
}

/// Structured result of one autonomous pass. The loop logs/skips these
/// outcomes; unlike interactive locate, it has no control connection and no
/// wire response to publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutolocateOutcome {
    Written,
    NoScanSnapshot,
    UnconfirmedScanSnapshot,
    StaleScanSnapshot,
    ScanClockFailure,
    NoStableCandidates,
    NoCachedPositions,
    Deduplicated,
    LastFixPreferred,
    Invalidated,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutolocateScanFreshness {
    Fresh,
    Absent,
    Unconfirmed,
    Stale { age: Duration },
}

#[derive(Debug)]
enum AutolocateScanClockError {
    Read(std::io::Error),
    FutureEvidence {
        now: BootTime,
        refresh_started_at: BootTime,
    },
}

impl std::fmt::Display for AutolocateScanClockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "failed to read CLOCK_BOOTTIME: {error}"),
            Self::FutureEvidence {
                now,
                refresh_started_at,
            } => write!(
                formatter,
                "confirmed refresh start {refresh_started_at:?} is ahead of current \
                 CLOCK_BOOTTIME {now:?}"
            ),
        }
    }
}

fn classify_autolocate_scan<F>(
    snapshot: Option<&TimestampedScan>,
    max_age_secs: u64,
    read_boot_time: F,
) -> Result<AutolocateScanFreshness, AutolocateScanClockError>
where
    F: FnOnce() -> std::io::Result<BootTime>,
{
    let Some(snapshot) = snapshot else {
        return Ok(AutolocateScanFreshness::Absent);
    };
    // Explicit compatibility escape hatch: a present sample is accepted
    // without confirmation or age inspection. Absence was rejected above.
    if max_age_secs == 0 {
        return Ok(AutolocateScanFreshness::Fresh);
    }
    let Some(refresh_started_at) = snapshot.confirmed_refresh_started_at else {
        return Ok(AutolocateScanFreshness::Unconfirmed);
    };
    let now = read_boot_time().map_err(AutolocateScanClockError::Read)?;
    let Some(age) = now.checked_duration_since(refresh_started_at) else {
        return Err(AutolocateScanClockError::FutureEvidence {
            now,
            refresh_started_at,
        });
    };
    let maximum = Duration::from_secs(max_age_secs);
    if age <= maximum {
        Ok(AutolocateScanFreshness::Fresh)
    } else {
        Ok(AutolocateScanFreshness::Stale { age })
    }
}

fn admit_autolocate_scan<F>(
    snapshot: Option<&TimestampedScan>,
    max_age_secs: u64,
    read_boot_time: F,
) -> Result<(), AutolocateOutcome>
where
    F: FnOnce() -> std::io::Result<BootTime>,
{
    match classify_autolocate_scan(snapshot, max_age_secs, read_boot_time) {
        Ok(AutolocateScanFreshness::Fresh) => Ok(()),
        Ok(AutolocateScanFreshness::Absent) => Err(AutolocateOutcome::NoScanSnapshot),
        Ok(AutolocateScanFreshness::Unconfirmed) => Err(AutolocateOutcome::UnconfirmedScanSnapshot),
        Ok(AutolocateScanFreshness::Stale { age }) => {
            debug!(
                ?age,
                max_age_secs, "autolocate skipped stale confirmed scan snapshot"
            );
            Err(AutolocateOutcome::StaleScanSnapshot)
        }
        Err(error) => {
            warn!("autolocate scan freshness check failed closed before cache access: {error}");
            Err(AutolocateOutcome::ScanClockFailure)
        }
    }
}

enum LocateOutcome {
    Interactive(String),
    Autolocate(AutolocateOutcome),
}

enum LocateClock {
    System,
    #[cfg(test)]
    Fixed(chrono::DateTime<chrono::Utc>),
}

#[derive(Clone, Copy)]
enum ScanAgeClock {
    System,
    #[cfg(test)]
    Fixed(BootTime),
}

impl ScanAgeClock {
    fn now(self) -> std::io::Result<BootTime> {
        match self {
            Self::System => BootTime::now(),
            #[cfg(test)]
            Self::Fixed(now) => Ok(now),
        }
    }
}

impl LocateClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        match self {
            Self::System => chrono::Utc::now(),
            #[cfg(test)]
            Self::Fixed(now) => *now,
        }
    }
}

fn locate_early_outcome(
    purpose: LocatePurpose,
    interactive: impl FnOnce() -> String,
    autonomous: AutolocateOutcome,
) -> LocateOutcome {
    match purpose {
        LocatePurpose::Interactive => LocateOutcome::Interactive(interactive()),
        LocatePurpose::Autolocate => LocateOutcome::Autolocate(autonomous),
    }
}

async fn handle_locate(state: &Arc<DaemonState>, ticket: crate::privacy::PrivacyTicket) -> String {
    match compute_locate(state, ticket, LocatePurpose::Interactive).await {
        LocateOutcome::Interactive(response) => response,
        LocateOutcome::Autolocate(_) => unreachable!("interactive locate returned auto outcome"),
    }
}

pub(crate) async fn run_autolocate_pass(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
) -> AutolocateOutcome {
    match compute_locate(state, ticket, LocatePurpose::Autolocate).await {
        LocateOutcome::Autolocate(outcome) => outcome,
        LocateOutcome::Interactive(_) => unreachable!("autolocate returned interactive outcome"),
    }
}

#[cfg(test)]
pub(crate) async fn run_autolocate_pass_at(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
    now: chrono::DateTime<chrono::Utc>,
) -> AutolocateOutcome {
    match compute_locate_at(state, ticket, LocatePurpose::Autolocate, now).await {
        LocateOutcome::Autolocate(outcome) => outcome,
        LocateOutcome::Interactive(_) => unreachable!("autolocate returned interactive outcome"),
    }
}

#[cfg(test)]
pub(crate) async fn run_autolocate_pass_at_boot_time(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
    now: chrono::DateTime<chrono::Utc>,
    boot_now: BootTime,
) -> AutolocateOutcome {
    match compute_locate_with_clock(
        state,
        ticket,
        LocatePurpose::Autolocate,
        LocateClock::Fixed(now),
        ScanAgeClock::Fixed(boot_now),
    )
    .await
    {
        LocateOutcome::Autolocate(outcome) => outcome,
        LocateOutcome::Interactive(_) => unreachable!("autolocate returned interactive outcome"),
    }
}

/// Trilateration ran over surviving solver membership. Carries the outcome
/// plus the metadata `compute_locate_*` needs to build the wire response and
/// run plausibility without re-deriving either.
struct SolvedLocate {
    /// Trilateration outcome: Ok position, or the failure to surface.
    result: anyhow::Result<crate::trilaterate::Position>,
    /// task-0094 consensus denominator: every finite non-mobile visible cache
    /// hit, independent of stability, RSSI, top-N and outlier survival.
    visible_cached: Vec<(f64, f64)>,
    /// Number of cache rows handed to trilateration (== solver membership).
    source_count: usize,
    /// Finite fixed cache hits admitted to this solve (coincides with
    /// `source_count`; locate is cache-only).
    cached_count: usize,
}

/// Outcome of the pure locate kernel (task-0116).
enum LocateKernel {
    /// No finite non-mobile cached position survived filtering. The caller
    /// owns the cold-start policy (interactive background resolve vs.
    /// autonomous `NoCachedPositions`).
    NoPositions,
    /// Trilateration was attempted over the surviving top-N membership.
    Solved(SolvedLocate),
}

/// Pure locate kernel (task-0116): filter cached rows into trilateration input
/// and solve. No mutex acquisition, no task spawn, no DB write — deterministic
/// in `(cached_aps, active, top_n)`, hence unit-testable against synthetic rows.
///
/// The cache READ deliberately stays with the caller: interactive locate
/// touches `last_seen` while autonomous locate is read-only, so that single
/// impure step is the one part of "cached lookup + filter + trilat" this kernel
/// excludes.
///
/// `visible_cached` (the consensus denominator) is built in the SAME pass as
/// `positioned_aps` (top-N solver membership) and must stay together: consensus
/// counts every finite non-mobile visible cache hit regardless of stability,
/// RSSI, top-N or outlier survival, so splitting the pass would change the
/// denominator. Persisted-mobile and non-finite eligibility both run before the
/// top-N cutoff so stronger ineligible rows cannot starve weaker fixed rows;
/// `trilaterate` independently re-applies these guards for callers that bypass
/// this path.
fn solve_locate_from_cache(
    cached_aps: &[crate::db::ApInfo],
    active: &std::collections::HashMap<&str, &Candidate>,
    top_n: usize,
) -> LocateKernel {
    use crate::trilaterate::{trilaterate, PositionedAp};

    let mut visible_cached = Vec::new();
    let mut positioned_aps = Vec::new();
    for ap in cached_aps {
        // task-0227: SQLite REAL columns can hold NaN/±Inf; a non-finite
        // coordinate would flow through to_unit_vec -> NaN vector -> the
        // `mag > 0` guards are false-on-NaN -> a "successful" (NaN, NaN) fix.
        // Skip such rows here and name the offending bssid so the poisoned
        // cache row is traceable.
        if !ap.provider_reported_lat.is_finite() || !ap.provider_reported_lon.is_finite() {
            warn!(
                "skipping AP {}: non-finite cached position ({}, {})",
                ap.bssid, ap.provider_reported_lat, ap.provider_reported_lon
            );
            continue;
        }
        if ap.is_mobile {
            continue;
        }

        // task-0122 AC#4/#6: exclude locally-administered (randomized client)
        // MACs and any malformed BSSID BEFORE the consensus tally and the
        // outlier pipeline (drop_isolated). A randomized MAC seen in three
        // cities is three devices wearing one address, not a moving AP; letting
        // a cluster of them reach drop_isolated would let them vouch for one
        // another as a fake stay. Placed here (not inside filter_outliers,
        // which operates on bssid-less PositionedAp) so a poisoner never
        // becomes solver membership OR a consensus denominator entry. This
        // subsumes the task-0093 SSID denylist.
        //
        // We parse the stored `String` bssid here at the use site rather than
        // storing a `Bssid`: making db::ApInfo.bssid a `Bssid` is the deferred
        // follow-up task-0269, which would let this become an infallible field
        // read instead of a `.parse()`.
        match ap.bssid.parse::<crate::bssid::Bssid>() {
            Ok(b) if b.is_locally_administered() => {
                debug!(
                    "skipping AP {}: locally-administered (randomized) MAC excluded from trilateration",
                    ap.bssid
                );
                continue;
            }
            Ok(_) => {}
            Err(e) => {
                warn!(
                    "skipping AP {}: malformed BSSID ({e}); excluded from trilateration",
                    ap.bssid
                );
                continue;
            }
        }

        // Consensus deliberately ignores stability, RSSI, top-N and eventual
        // outlier-filter survival. Every eligible unique BSSID in the latest
        // scanner snapshot contributes exactly one denominator entry.
        visible_cached.push((ap.provider_reported_lat, ap.provider_reported_lon));
        // Single lookup (task-0264): membership and the signal come from the
        // same `active` entry, so `signal_dbm: Some(candidate.signal_dbm)` states
        // the always-present invariant honestly. Edition 2021 has no let-chains,
        // hence the nested top-N `if`.
        if let Some(candidate) = active.get(ap.bssid.as_str()) {
            if positioned_aps.len() < top_n {
                positioned_aps.push(PositionedAp {
                    lat: ap.provider_reported_lat,
                    lon: ap.provider_reported_lon,
                    signal_dbm: Some(candidate.signal_dbm),
                    // Provenance threaded from the scanner (task-0126); the dBm
                    // value itself is unchanged.
                    rssi_source: candidate.rssi_source,
                    is_mobile: ap.is_mobile,
                });
            }
        }
    }

    if positioned_aps.is_empty() {
        return LocateKernel::NoPositions;
    }

    // `cached` counts only finite fixed cache hits admitted to this solve, not
    // every row examined while filling top-N. `sources` is the number handed to
    // trilateration; locate is cache-only, so they coincide for fresh fixes.
    let cached_count = positioned_aps.len();
    let source_count = positioned_aps.len();
    let result = trilaterate(&positioned_aps);
    LocateKernel::Solved(SolvedLocate {
        result,
        visible_cached,
        source_count,
        cached_count,
    })
}

async fn compute_locate(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
    purpose: LocatePurpose,
) -> LocateOutcome {
    compute_locate_with_clock(
        state,
        ticket,
        purpose,
        LocateClock::System,
        ScanAgeClock::System,
    )
    .await
}

/// Shared locate pipeline. `now` is injected for deterministic cadence and
/// dedup tests; production callers pass one high-resolution UTC wall clock.
#[cfg(test)]
async fn compute_locate_at(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
    purpose: LocatePurpose,
    now: chrono::DateTime<chrono::Utc>,
) -> LocateOutcome {
    compute_locate_with_clock(
        state,
        ticket,
        purpose,
        LocateClock::Fixed(now),
        ScanAgeClock::System,
    )
    .await
}

async fn compute_locate_with_clock(
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
    purpose: LocatePurpose,
    clock: LocateClock,
    scan_age_clock: ScanAgeClock,
) -> LocateOutcome {
    use crate::resolver;

    let Some(privacy) = state.privacy.read(ticket).await else {
        return locate_early_outcome(
            purpose,
            || error_json(STALE_AFTER_WIPE_ERROR),
            AutolocateOutcome::Invalidated,
        );
    };
    let debouncer = state.debouncer.lock().await;
    // TASK-0246: freshness admission is the first autonomous operation under
    // the privacy + Debouncer guards. It must precede stability calculation,
    // candidate cloning, every DB/cache actor call, plausibility, last_fix and
    // persistence. Interactive locate deliberately retains legacy behavior.
    if purpose == LocatePurpose::Autolocate {
        if let Err(outcome) = admit_autolocate_scan(
            debouncer.latest_snapshot(),
            state.args.autolocate_max_scan_age_secs,
            || scan_age_clock.now(),
        ) {
            return LocateOutcome::Autolocate(outcome);
        }
    }
    let stable = debouncer.stable_bssids();
    let visible = debouncer.latest_scan().map(|s| s.len()).unwrap_or(0);

    // Snapshot every scanner-eligible AP in the latest scan exactly once.
    // This full set is the consensus universe; solver membership is derived
    // separately below so stability and top-N never shrink the denominator.
    let stable_count = stable.len();
    let mut visible_candidates: Vec<Candidate> = debouncer
        .latest_scan()
        .map(|scan| {
            scan.iter()
                .filter(|(_, entry)| {
                    !crate::scanner::is_mobile_ssid(
                        entry.ssid.as_deref(),
                        &state.args.mobile_ssid_denylist,
                    )
                })
                .map(|(bssid, entry)| Candidate {
                    bssid: bssid.clone(),
                    signal_dbm: entry.signal_dbm,
                    rssi_source: entry.rssi_source,
                    ssid: entry.ssid.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    visible_candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.signal_dbm));

    // Stable BSSIDs that fell out of the current scan are absent from this
    // list instead of receiving a fabricated -90 dBm signal (task-0051).
    let stable_candidates: Vec<Candidate> = visible_candidates
        .iter()
        .filter(|candidate| stable.contains(&candidate.bssid))
        .cloned()
        .collect();

    drop(debouncer);
    drop(privacy);

    // Interactive locate retains its cold-start fallback to unstable visible
    // APs. Autonomous history is deliberately stable-only: it consumes the
    // scanner-owned Debouncer snapshot and never initiates a scan or provider
    // lookup itself.
    if purpose == LocatePurpose::Autolocate && stable_candidates.is_empty() {
        return LocateOutcome::Autolocate(AutolocateOutcome::NoStableCandidates);
    }
    let using_fallback = stable_candidates.is_empty() && !visible_candidates.is_empty();
    let active_candidates = if stable_candidates.is_empty() {
        &visible_candidates
    } else {
        &stable_candidates
    };

    if active_candidates.is_empty() {
        let error = if visible == 0 {
            "no APs detected yet"
        } else {
            "no eligible fixed APs detected"
        };
        return match purpose {
            LocatePurpose::Interactive => LocateOutcome::Interactive(
                return_last_fix_or_error(state, ticket, error, visible, stable_count).await,
            ),
            LocatePurpose::Autolocate => {
                LocateOutcome::Autolocate(AutolocateOutcome::NoStableCandidates)
            }
        };
    }

    // Both purposes read every scanner-eligible AP in the latest snapshot so
    // TASK-0094 consensus keeps its full-visible denominator. Autonomous
    // solver membership remains stable-only through the `active` index below.
    let bssids: Vec<String> = visible_candidates
        .iter()
        .map(|candidate| candidate.bssid.clone())
        .collect();

    // Cache-only lookup — never blocks on WiGLE. This is the sole cache lookup
    // for locate and preserves full-snapshot RSSI order even when absent
    // BSSIDs are omitted. Interactive locate refreshes AP observation age;
    // autonomous locate is read-only unless it later commits a fix.
    let cached_aps = match purpose {
        LocatePurpose::Interactive => resolver::lookup_cached(&bssids, state, ticket).await,
        LocatePurpose::Autolocate => resolver::lookup_cached_readonly(&bssids, state, ticket).await,
    };

    // Index the active candidate set by BSSID so the pure kernel can resolve
    // solver membership and per-AP signal from one lookup. The impure cache
    // READ above is deliberately outside the kernel (see `solve_locate_from_cache`).
    let active: std::collections::HashMap<&str, &Candidate> = active_candidates
        .iter()
        .map(|candidate| (candidate.bssid.as_str(), candidate))
        .collect();

    let (result, visible_cached, source_count, cached_count) =
        match solve_locate_from_cache(&cached_aps, &active, state.args.top_n) {
            LocateKernel::NoPositions => {
                if purpose == LocatePurpose::Autolocate {
                    return LocateOutcome::Autolocate(AutolocateOutcome::NoCachedPositions);
                }
                // Cold-start provider work must stay bounded even though cache
                // lookup examines every scanner-eligible candidate to backfill
                // top-N around persisted-mobile/non-finite rows. Cached rows
                // cannot benefit from the skip-cached provider chain, so omit
                // them before taking the limit; this prevents stronger invalid
                // cache hits from starving unknown fixed APs.
                let cached_bssids: std::collections::HashSet<String> =
                    cached_aps.iter().map(|ap| ap.bssid.clone()).collect();
                let unresolved_observations: Vec<resolver::ObservedAp> = if using_fallback {
                    bounded_uncached_observations(
                        active_candidates,
                        &cached_bssids,
                        state.args.top_n,
                    )
                } else {
                    Vec::new()
                };
                let resolving_in_background = !unresolved_observations.is_empty();
                if resolving_in_background {
                    // task-0102: track so the shutdown drain waits for this
                    // resolve instead of killing it mid-API-call (which would
                    // leak a WiGLE quota lease / leave partial writes).
                    let resolve_state = Arc::clone(state);
                    state.spawn_tracked(async move {
                        resolver::resolve_background(
                            &unresolved_observations,
                            &resolve_state,
                            ticket,
                        )
                        .await;
                    });
                }
                let error = if resolving_in_background {
                    "no cached positions for visible APs (resolving in background)"
                } else {
                    "no eligible cached positions for visible APs"
                };
                return LocateOutcome::Interactive(
                    return_last_fix_or_error(state, ticket, error, visible, stable_count).await,
                );
            }
            LocateKernel::Solved(solved) => (
                solved.result,
                solved.visible_cached,
                solved.source_count,
                solved.cached_count,
            ),
        };

    match result {
        Ok(pos) => {
            // The complete policy boundary is serialized in the established
            // privacy -> last_fix -> database order. Read the previous row
            // literally from SQLite while holding last_fix, then capture one
            // high-resolution wall clock for age, speed, decision and any
            // current-winner persistence. Concurrent locate/backfill calls
            // cannot change the previous row between these phases.
            let Some(_privacy) = state.privacy.read(ticket).await else {
                return locate_early_outcome(
                    purpose,
                    || error_json(STALE_AFTER_WIPE_ERROR),
                    AutolocateOutcome::Invalidated,
                );
            };
            let mut last = state.last_fix.lock().await;
            // task-0134: get_last_fix now returns the domain `LastFix` with the
            // Row->domain guard applied. A poisoned persisted row surfaces as a
            // `CorruptLastFix` error, distinct from a genuine read failure, so
            // this preserves the pre-refactor split: ignore a corrupt row and
            // still answer; abort only on an actual SQLite read failure.
            let previous = match state.db.get_last_fix().await {
                Ok(previous) => previous,
                // This downcast depends on `CorruptLastFix` remaining the
                // top-level anyhow error out of `get_last_fix` (do NOT
                // `.context(..)`-wrap it in the DB layer — see the invariant on
                // `last_fix_from_columns`). If it breaks, a poisoned row would
                // fall through to the abort arm below instead of being ignored;
                // `locate_with_corrupt_persisted_last_fix_still_answers` pins it.
                Err(error) if error.downcast_ref::<crate::db::CorruptLastFix>().is_some() => {
                    warn!("ignoring invalid persisted last_fix during locate: {error}");
                    None
                }
                Err(error) => {
                    if purpose == LocatePurpose::Autolocate {
                        warn!("autolocate failed to read persisted last_fix: {error}");
                    }
                    return locate_early_outcome(
                        purpose,
                        || error_json(&format!("failed to read last_fix: {error}")),
                        AutolocateOutcome::Failed,
                    );
                }
            };
            let now = clock.now();
            if purpose == LocatePurpose::Autolocate
                && previous.as_ref().is_some_and(|previous| {
                    autolocate_is_duplicate(
                        previous,
                        (pos.lat, pos.lon),
                        now,
                        state.args.history_segment_distance_m,
                        state.args.autolocate_interval_secs,
                    )
                })
            {
                return LocateOutcome::Autolocate(AutolocateOutcome::Deduplicated);
            }
            let decision = evaluate_plausibility(
                previous.as_ref(),
                (pos.lat, pos.lon),
                &visible_cached,
                now,
                state.args.max_travel_speed_kmh,
            );

            if decision.quality.confidence() == Confidence::Suspect {
                let (winner, loser) = match decision.winner {
                    PlausibilityWinner::CurrentTrilat => ("current_trilat", "last_fix"),
                    PlausibilityWinner::LastFix => ("last_fix", "current_trilat"),
                };
                warn!(
                    current_trilat_score = ?decision.current_score,
                    last_fix_score = ?decision.last_fix_score,
                    "plausibility tie-break rejected {loser}; favored {winner}"
                );
            }

            let (lat, lon, accuracy_m, address, fallback) = match decision.winner {
                PlausibilityWinner::CurrentTrilat => {
                    let mut new_fix = LastFix {
                        lat: pos.lat,
                        lon: pos.lon,
                        accuracy_m: pos.accuracy_m,
                        address: None,
                        at: now,
                        sources: source_count,
                    };

                    if purpose == LocatePurpose::Autolocate {
                        if state.args.address_approx {
                            let cache = state.address_cache.lock().unwrap_or_else(|error| {
                                warn!("address cache mutex poisoned — recovering");
                                error.into_inner()
                            });
                            new_fix.address = cache.get(new_fix.lat, new_fix.lon);
                        }
                        let last_fix_applied = if pos.fallback {
                            // task-0263: an antipodal-fallback fix carries the
                            // accuracy_m=1000 "position unknown" sentinel. Do NOT
                            // let it overwrite the authoritative last_fix — a later
                            // stale read would echo ~1000 m with fallback=false,
                            // re-introducing the in-band ambiguity task-0127
                            // removed. Still record the observation in the fixes
                            // timeseries (history is separate from last_fix). Treat
                            // it as "not applied" so memory realigns to `previous`
                            // via the shared seam below.
                            if let Err(error) = state
                                .db
                                .persist_history_only(new_fix.clone(), decision.quality.clone())
                                .await
                            {
                                warn!("failed to persist autonomous fallback history: {error}");
                                state.record_db_failure();
                                return LocateOutcome::Autolocate(AutolocateOutcome::Failed);
                            }
                            false
                        } else {
                            match state
                                .db
                                .persist_autolocate_fix_and_history(
                                    new_fix.clone(),
                                    decision.quality.clone(),
                                )
                                .await
                            {
                                Ok(applied) => applied,
                                Err(error) => {
                                    warn!(
                                        "failed to persist autonomous last_fix and history: {error}"
                                    );
                                    state.record_db_failure();
                                    return LocateOutcome::Autolocate(AutolocateOutcome::Failed);
                                }
                            }
                        };
                        // Commit is the publication boundary for autonomous
                        // work. A backwards wall-clock observation still
                        // appends history, but SQLite retains the newer
                        // authoritative last_fix and memory must not regress.
                        if last_fix_applied {
                            *last = Some(new_fix);
                        } else {
                            // SQLite kept a newer wall-clock row. Re-align
                            // memory to that validated persisted authority
                            // instead of publishing the older observation.
                            *last = previous.clone();
                        }
                        state
                            .autolocate_fixes_written
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        return LocateOutcome::Autolocate(AutolocateOutcome::Written);
                    }

                    let address = if pos.fallback {
                        // task-0263: skip advancing last_fix for a fallback fix.
                        // Its accuracy_m=1000 sentinel means "position unknown";
                        // caching it would later be served on the stale path as a
                        // ~1000 m position with fallback=false. Leave the prior
                        // last_fix (memory + disk) intact and record the
                        // observation in the fixes timeseries only. No geocode: a
                        // sentinel position has no meaningful address.
                        //
                        // NOTE: this arm relies on `*last` (the in-memory
                        // last_fix) NOT being mutated earlier in this CurrentTrilat
                        // branch — unlike the autolocate arm, it does not
                        // re-assign `*last = previous`. Do not add a `*last`
                        // write ahead of this point, or a fallback fix would
                        // silently overwrite the prior in-memory last_fix.
                        if let Err(error) = state
                            .db
                            .persist_history_only(new_fix, decision.quality.clone())
                            .await
                        {
                            warn!("failed to persist fallback history: {error}");
                            state.record_db_failure();
                        }
                        None
                    } else if state.args.address_approx {
                        persist_locate_fix_and_schedule_locked(
                            state,
                            ticket,
                            &mut last,
                            new_fix,
                            decision.quality.clone(),
                        )
                        .await
                    } else {
                        persist_last_fix_and_history_locked(
                            state,
                            &mut last,
                            new_fix,
                            decision.quality.clone(),
                        )
                        .await;
                        None
                    };
                    (pos.lat, pos.lon, pos.accuracy_m, address, pos.fallback)
                }
                PlausibilityWinner::LastFix => {
                    if purpose == LocatePurpose::Autolocate {
                        return LocateOutcome::Autolocate(AutolocateOutcome::LastFixPreferred);
                    }
                    let previous = previous
                        .as_ref()
                        .expect("a last_fix winner requires a validated previous row");
                    // Do not rewrite last_fix/history here. The prior row is
                    // already the literal winning observation; refreshing its
                    // timestamp would invent motion evidence, and persisting
                    // the drift-bound accuracy would compound it on every call.
                    // The reported position is the persisted previous fix, a
                    // legitimate observation. LastFix does not carry the
                    // response-only `fallback` signal, so it is always false
                    // here regardless of the current trilateration's fallback.
                    (
                        previous.lat,
                        previous.lon,
                        previous.accuracy_m + decision.distance_m,
                        previous.address.clone(),
                        false,
                    )
                }
            };
            drop(last);

            let resp = LocateResponse {
                ok: true,
                v: WIRE_VERSION,
                lat,
                lon,
                accuracy_m,
                sources: source_count,
                cached: cached_count,
                visible,
                stable: stable_count,
                address,
                stale: false,
                fallback,
                age_s: None,
                implied_speed_kmh: decision.quality.implied_speed_kmh(),
                last_fix_age_secs: decision.quality.last_fix_age_secs(),
                inlier_ratio: decision.quality.inlier_ratio(),
                confidence: decision.quality.confidence(),
                suspect_reason: decision.quality.suspect_reason().map(str::to_owned),
            };
            LocateOutcome::Interactive(ok_json(&resp))
        }
        Err(e) => match purpose {
            LocatePurpose::Interactive => LocateOutcome::Interactive(
                return_last_fix_or_error(
                    state,
                    ticket,
                    &format!("trilateration failed: {e}"),
                    visible,
                    stable_count,
                )
                .await,
            ),
            LocatePurpose::Autolocate => {
                warn!("autolocate trilateration failed: {e}");
                LocateOutcome::Autolocate(AutolocateOutcome::Failed)
            }
        },
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
        // Wall-clock age. Negative ages (clock skew) clamp to zero and remain
        // observable instead of silently masquerading as a fresh timestamp.
        let now = chrono::Utc::now();
        if now < fix.at {
            warn!(
                previous_at = %fix.at,
                now = %now,
                "stale last_fix timestamp is in the future; clamping reported age to zero"
            );
        }
        let age = (now - fix.at).num_seconds().max(0);
        let age_s = age as u64;
        let resp = LocateResponse {
            ok: true,
            v: WIRE_VERSION,
            lat: fix.lat,
            lon: fix.lon,
            accuracy_m: fix.accuracy_m,
            sources: fix.sources,
            cached: 0,
            visible,
            stable,
            address: fix.address.clone(),
            stale: true,
            fallback: false,
            age_s: Some(age_s),
            implied_speed_kmh: None,
            last_fix_age_secs: Some(age),
            inlier_ratio: None,
            confidence: Confidence::Low,
            suspect_reason: None,
        };
        ok_json(&resp)
    } else {
        error_json(error_msg)
    }
}

/// Normalize and validate the BSSIDs of a resolve request (task-0122 AC#9).
///
/// Returns the canonical BSSID strings on success, or an error message naming
/// the first malformed entry. Pure and network-free so the "no malformed BSSID
/// reaches a provider" guarantee is unit-testable without a `DaemonState`.
/// Locally-administered MACs are well-formed and accepted here — resolving an
/// individual randomized MAC is a valid (if rarely useful) query; the
/// randomized-MAC exclusion is a trilateration concern, not a resolve concern.
fn validate_resolve_bssids(bssids: &[String]) -> Result<Vec<String>, String> {
    bssids
        .iter()
        .map(|b| {
            let normalized = crate::scanner::normalize_bssid(b);
            normalized
                .parse::<crate::bssid::Bssid>()
                .map(|_| normalized)
                .map_err(|e| format!("malformed BSSID {b:?}: {e}"))
        })
        .collect()
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

    // task-0122 AC#9: reject any BSSID that is not a well-formed 6-octet MAC
    // BEFORE any provider/network call, so a malformed value can never reach a
    // provider or the outbound WiGLE/Apple request.
    let normalized = match validate_resolve_bssids(bssids) {
        Ok(normalized) => normalized,
        Err(msg) => return error_json(&msg),
    };

    let result = resolver::resolve_readonly(&normalized, state, ticket).await;

    let results: Vec<ResolveResultEntry> = normalized
        .iter()
        .map(|bssid| {
            match result.positioned.iter().find(|a| &a.bssid == bssid) {
                Some(ap)
                    if ap.provider_reported_lat.is_finite()
                        && ap.provider_reported_lon.is_finite() =>
                {
                    ResolveResultEntry {
                        bssid: bssid.clone(),
                        lat: Some(ap.provider_reported_lat),
                        lon: Some(ap.provider_reported_lon),
                        ssid: ap.ssid.clone(),
                        provenance: if result.fetched_bssids.contains(bssid) {
                            Provenance::Api
                        } else {
                            Provenance::Cache
                        },
                    }
                }
                // task-0238: fail closed at the protocol boundary. A poisoned
                // cache row is not authoritative absence, so classify it as
                // unresolved (retryable), never not_found. This guard also
                // protects against a future provider returning non-finite
                // coordinates directly in this read-only path.
                Some(ap) => {
                    warn!(
                        "discarding resolve position for {}: non-finite coordinates ({}, {})",
                        ap.bssid, ap.provider_reported_lat, ap.provider_reported_lon
                    );
                    ResolveResultEntry {
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
                    ResolveResultEntry {
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
        v: WIRE_VERSION,
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
            let mut nets: Vec<NetworkEntry> = sample
                .iter()
                // Wire boundary: the raw i32 dBm crosses, and task-0271 now also
                // surfaces `entry.rssi_source` (iw authoritative vs nmcli biased
                // linearization) as an additive optional wire field so an
                // operator can tell an authoritative reading from a guess.
                .map(|(bssid, entry)| NetworkEntry {
                    bssid: bssid.clone(),
                    ssid: entry.ssid.clone(),
                    signal_dbm: entry.signal_dbm,
                    channel: entry.channel,
                    rssi_source: Some(entry.rssi_source.into()),
                })
                .collect();
            nets.sort_by_key(|n| std::cmp::Reverse(n.signal_dbm));
            nets
        }
        None => Vec::new(),
    };

    let resp = ScanResponse {
        ok: true,
        v: WIRE_VERSION,
        networks,
        scan_age_ms,
        scanned_at,
    };
    ok_json(&resp)
}

/// One debug-wire input row assembled before DB status is joined in:
/// `(bssid, signal_dbm, rssi_source, seen, is_stable)`. `signal_dbm` and
/// `rssi_source` are `Some` together (a scanned BSSID) or `None` together (a
/// stable-but-absent BSSID). task-0271 added the provenance element.
type DebugInputRow = (
    String,
    Option<i32>,
    Option<whereami_protocol::RssiSource>,
    usize,
    bool,
);

async fn handle_debug(state: &Arc<DaemonState>) -> String {
    let debouncer = state.debouncer.lock().await;
    let scan_age_ms = debouncer.latest_scan_age_ms();
    let samples = debouncer.sample_count();
    let threshold = debouncer.threshold();
    let stable_set = debouncer.stable_bssids();

    // Collect ALL BSSIDs ever seen in the ring buffer. Signals from the
    // latest scan are Some((dbm, source)); stable BSSIDs that fell out of the
    // latest scan are None (task-0051). task-0271 carries the RSSI provenance
    // (iw vs nmcli-linearized) alongside the dBm so it can cross the wire.
    let mut all_bssids: std::collections::HashMap<
        String,
        Option<(i32, whereami_protocol::RssiSource)>,
    > = std::collections::HashMap::new();
    if let Some(scan) = debouncer.latest_scan() {
        for (bssid, entry) in scan {
            all_bssids.insert(
                bssid.clone(),
                Some((entry.signal_dbm, entry.rssi_source.into())),
            );
        }
    }
    // Also include stable ones not in latest scan with signal=None.
    for b in &stable_set {
        all_bssids.entry(b.clone()).or_insert(None);
    }

    let visible = debouncer.latest_scan().map(|s| s.len()).unwrap_or(0);

    let debug_inputs: Vec<DebugInputRow> = all_bssids
        .iter()
        .map(|(bssid, signal)| {
            (
                bssid.clone(),
                signal.map(|(dbm, _)| dbm),
                signal.map(|(_, source)| source),
                debouncer.count(bssid),
                stable_set.contains(bssid),
            )
        })
        .collect();
    drop(debouncer);

    let statuses = match state
        .db
        .debug_statuses(
            debug_inputs.iter().map(|item| item.0.clone()).collect(),
            state.args.not_found_ttl_days,
        )
        .await
    {
        Ok(statuses) => statuses,
        Err(e) => {
            // A read error would silently mislabel every BSSID's DB status on
            // the debug surface. Count it so the fault is visible (task-0252).
            warn!("db error reading debug statuses (all reported as unknown): {e}");
            state.record_db_read_failure();
            Default::default()
        }
    };
    let statuses: std::collections::HashMap<_, _> = statuses.into_iter().collect();
    let mut bssids: Vec<DebugBssid> = debug_inputs
        .into_iter()
        .map(|(bssid, signal_dbm, rssi_source, seen, is_stable)| {
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
                rssi_source,
            }
        })
        .collect();
    // Sort: BSSIDs with a current signal first (strongest first); BSSIDs
    // without a current signal last (None sorts after Some(_) under Reverse).
    bssids.sort_by_key(|n| std::cmp::Reverse(n.signal_dbm));

    let stable_count = stable_set.len();
    let resp = DebugResponse {
        ok: true,
        v: WIRE_VERSION,
        // The shared wire type carries `Option<String>` (an old daemon could
        // omit it); the daemon always emits its compiled-in revision, so the
        // bytes are unchanged.
        daemon_rev: Some(env!("GIT_REV").to_string()),
        scan_age_ms,
        samples_in_buffer: samples,
        visible,
        stable: stable_count,
        bssids,
    };
    ok_json(&resp)
}

fn handle_version() -> String {
    let resp = VersionResponse {
        ok: true,
        v: WIRE_VERSION,
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        git_rev: Some(env!("GIT_REV").to_string()),
    };
    ok_json(&resp)
}

/// Build the shared wire [`HistoryFix`] from a persisted DB row. The conversion
/// lives here (not in the protocol crate) because it depends on daemon-internal
/// types (`crate::db::Fix`, `crate::quality::FixQuality`); coherence permits it
/// because `db::Fix` is local even though `HistoryFix` is foreign. The daemon
/// thus serializes the shared type directly — there is no daemon-private
/// raw-history struct to drift from the client's (task-0114).
impl From<crate::db::Fix> for HistoryFix {
    fn from(row: crate::db::Fix) -> Self {
        let (implied_speed_kmh, last_fix_age_secs, inlier_ratio, confidence, suspect_reason) = row
            .quality
            .map_or((None, None, None, None, None), |quality| {
                (
                    quality.implied_speed_kmh(),
                    quality.last_fix_age_secs(),
                    quality.inlier_ratio(),
                    Some(quality.confidence()),
                    quality.suspect_reason().map(str::to_owned),
                )
            });
        Self {
            at_rfc3339: row.at.to_rfc3339(),
            lat: row.lat,
            lon: row.lon,
            accuracy_m: row.accuracy_m,
            n_sources: row.n_sources,
            implied_speed_kmh,
            last_fix_age_secs,
            inlier_ratio,
            confidence,
            suspect_reason,
        }
    }
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
    fixes: Vec<crate::db::Fix>,
    from: String,
    to: String,
    output: HistoryOutput,
    shutdown: tokio_util::sync::CancellationToken,
    lease: Arc<tokio::sync::OwnedSemaphorePermit>,
}

enum HistoryOutput {
    Segmented {
        dist_threshold_m: f64,
        min_duration_secs: i64,
    },
    Raw {
        elided: Option<i64>,
    },
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
        output,
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

        let body = match output {
            HistoryOutput::Segmented {
                dist_threshold_m,
                min_duration_secs,
            } => {
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
                HistoryBody::Segmented(segments)
            }
            HistoryOutput::Raw { elided } => HistoryBody::Raw {
                fixes: fixes.into_iter().map(HistoryFix::from).collect(),
                elided,
            },
        };
        if blocking_cancelled.load(std::sync::atomic::Ordering::Acquire) || shutdown.is_cancelled()
        {
            return error_json("history: cancelled");
        }

        match body {
            HistoryBody::Segmented(segments) => ok_json(&HistoryResponse {
                ok: true,
                v: WIRE_VERSION,
                from,
                to,
                segments,
            }),
            HistoryBody::Raw { fixes, elided } => ok_json(&HistoryFixesResponse {
                ok: true,
                v: WIRE_VERSION,
                range: HistoryRange {
                    from_rfc3339: from,
                    to_rfc3339: to,
                },
                fixes,
                elided,
            }),
        }
    });
    (join, guard)
}

enum HistoryBody {
    Segmented(Vec<crate::history::Segment>),
    Raw {
        fixes: Vec<HistoryFix>,
        elided: Option<i64>,
    },
}

async fn handle_history(
    range: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    raw: bool,
    max_fixes: Option<u64>,
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
) -> CommandResponse {
    // Resolve range: either `range="7d"` shorthand OR explicit from/to RFC3339.
    // Mutually exclusive. If neither is provided, default to the last 7 days.
    let (from, to) = match (range, from, to) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            return error_json("history: 'range' and 'from'/'to' are mutually exclusive").into();
        }
        (Some(spec), None, None) => match crate::duration_spec::parse_range(spec) {
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
        (None, None, None) => crate::duration_spec::parse_range("7d")
            .expect("hardcoded default range '7d' must parse"),
        _ => {
            return error_json("history: provide either 'range' or both 'from' and 'to'").into();
        }
    };

    let raw_limit = match (raw, max_fixes) {
        (false, Some(_)) => {
            return error_json("history: 'max_fixes' requires 'raw': true").into();
        }
        (true, limit) => {
            let limit = limit.unwrap_or(DEFAULT_RAW_HISTORY_MAX_FIXES);
            if !(1..=MAX_RAW_HISTORY_FIXES).contains(&limit) {
                return error_json(&format!(
                    "history: 'max_fixes' must be in 1..={MAX_RAW_HISTORY_FIXES}"
                ))
                .into();
            }
            Some(limit)
        }
        (false, None) => None,
    };

    // Never queue history waiters behind the expensive request: they would
    // consume connection slots and let a burst starve cheap commands.
    let lease = match Arc::clone(&state.history_slot).try_acquire_owned() {
        Ok(lease) => Arc::new(lease),
        Err(_) => return error_json("history: busy; retry later").into(),
    };

    let from = from.to_rfc3339();
    let to = to.to_rfc3339();
    let (fixes, output) = if let Some(limit) = raw_limit {
        match state
            .db
            .get_capped_fixes_in_range_guarded(from.clone(), to.clone(), limit, Arc::clone(&lease))
            .await
        {
            Ok(page) => (
                page.fixes,
                HistoryOutput::Raw {
                    elided: page.elided,
                },
            ),
            Err(error) => {
                return CommandResponse::history(
                    error_json(&format!("history: db error: {error}")),
                    lease,
                    ticket,
                );
            }
        }
    } else {
        // Segmentation ignores per-fix quality, but it reads through the same
        // quality-decoding projection, so a corrupt quality column in ANY row in
        // range fails the whole (position-only) segmented view, not just raw
        // history. This is a deliberate fail-loud choice: production only ever
        // writes validated bundles (FixQuality::new), the DB is single-writer
        // 0600, and silently serving positions from a DB with detectably-corrupt
        // rows would violate the fail-fast contract. task-0247 tracks an optional
        // position-only projection if this blast radius ever needs narrowing.
        match state
            .db
            .get_fixes_in_range_guarded(from.clone(), to.clone(), Arc::clone(&lease))
            .await
        {
            Ok(fixes) => (
                fixes,
                HistoryOutput::Segmented {
                    dist_threshold_m: state.args.history_segment_distance_m as f64,
                    min_duration_secs: state.args.history_segment_min_duration_secs as i64,
                },
            ),
            Err(error) => {
                return CommandResponse::history(
                    error_json(&format!("history: db error: {error}")),
                    lease,
                    ticket,
                );
            }
        }
    };
    let (join, mut cancellation) = spawn_history_work(
        HistoryWork {
            fixes,
            from,
            to,
            output,
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
                v: WIRE_VERSION,
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
    let autolocate_fixes_written = state
        .autolocate_fixes_written
        .load(std::sync::atomic::Ordering::Relaxed);
    let orphan_tasks_aborted = state
        .orphan_tasks_aborted
        .load(std::sync::atomic::Ordering::Relaxed);
    // A stats read error silently zeroes cached_aps/pending_aps/not_found_aps/
    // db_size_bytes (the row counts would lie). Count it so the fault is visible
    // via db_read_failures, then fall back to defaults (task-0252).
    let stats = match state.db.stats().await {
        Ok(stats) => stats,
        Err(e) => {
            warn!("db error reading stats (row counts fall back to 0): {e}");
            state.record_db_read_failure();
            crate::db::DbStats::default()
        }
    };
    // Snapshot db_read_failures AFTER the stats read so a stats-read fault this
    // call is reflected in THIS response (self-consistent), not only the next.
    let db_read_failures = state
        .db_read_failures
        .load(std::sync::atomic::Ordering::Relaxed);
    let resp = StatsResponse {
        ok: true,
        v: WIRE_VERSION,
        cached_aps: stats.cached_aps,
        pending_aps: stats.pending_aps,
        not_found_aps: stats.not_found_aps,
        db_size_bytes: stats.db_size_bytes,
        wigle_lookups_today: stats.wigle_lookups_today,
        // Transitional dual-emit (task-0130): mirror the SAME value into the
        // deprecated api_calls_today field so pre-rename clients keep rendering
        // the WiGLE daily count. Remove in task-0267 once clients read the new
        // field.
        api_calls_today: stats.wigle_lookups_today,
        wigle_counter_corruption_denials: api_counter_corruption_denials,
        // Transitional dual-emit (task-0268): mirror the SAME value into the
        // deprecated api_counter_corruption_denials field so pre-rename clients
        // keep rendering the WiGLE corruption-denial count. Remove in task-0275
        // once clients read wigle_counter_corruption_denials.
        api_counter_corruption_denials,
        inflight,
        db_write_failures,
        db_read_failures,
        autolocate_fixes_written,
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
    test_state_with_db(
        db_runtime,
        db,
        test_db,
        crate::wigle::WigleClient::new("", ""),
        nominatim,
    )
}

/// task-0230: single shared test-only DaemonState constructor with a
/// customizable WiGLE client, opening its own in-memory test DB. The
/// provider/pending/resolver test modules route through this (and `test_state`)
/// instead of hand-writing their own struct literals, so a new DaemonState
/// field is added in exactly one place — `test_state_with_db` below.
#[cfg(test)]
pub(crate) fn test_state_with_wigle_client(
    wigle: crate::wigle::WigleClient,
) -> std::sync::Arc<DaemonState> {
    let (db_runtime, db, test_db) = crate::db::DbRuntime::open_test().unwrap();
    test_state_with_db(
        db_runtime,
        db,
        test_db,
        wigle,
        crate::nominatim::NominatimClient::new(),
    )
}

#[cfg(test)]
fn test_state_with_db(
    db_runtime: crate::db::DbRuntime,
    db: crate::db::DbHandle,
    test_db: crate::db::Database,
    wigle: crate::wigle::WigleClient,
    nominatim: crate::nominatim::NominatimClient,
) -> std::sync::Arc<DaemonState> {
    use clap::Parser;
    // Args defaults set debounce_window=10, debounce_threshold=5 — the exact
    // values the former provider/pending/resolver literals hardcoded, so the
    // args-derived debouncer here is behavior-identical to those call sites.
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
        wigle,
        apple: crate::apple::AppleClient::new(),
        nominatim,
        geocode: geocode::GeocodeScheduler::new(shutdown.clone()),
        last_fix: tokio::sync::Mutex::new(None),
        inflight: std::sync::Mutex::new(std::collections::HashMap::new()),
        address_cache: std::sync::Mutex::new(AddressCache::new()),
        db_write_failures: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        db_read_failures: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        autolocate_fixes_written: Arc::new(std::sync::atomic::AtomicU64::new(0)),
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
            "wigle_lookups_today` can render its fallback `0`",
            "does not change protocol version `v: 1`",
        ] {
            assert!(
                protocol.contains(marker),
                "protocol is missing semantic marker {marker:?}"
            );
        }
    }

    #[test]
    fn protocol_documents_autolocate_counter_semantics() {
        let protocol = include_str!("../../docs/protocol.md");
        for marker in [
            "\"autolocate_fixes_written\": 0",
            "autonomous history rows successfully",
            "Manual `locate` writes are excluded",
            "resets on restart",
            "protocol version remains `v: 1`",
        ] {
            assert!(
                protocol.contains(marker),
                "protocol is missing autolocate marker {marker:?}"
            );
        }
    }

    #[tokio::test]
    async fn stats_wire_reports_corruption_denials_while_counter_falls_back_to_zero() {
        let state = test_state();
        {
            let db = lock_db(&state);
            db.set_metadata("wigle_lookups_today", "malformed").unwrap();
        }
        state
            .api_counter_corruption_denials
            .store(3, std::sync::atomic::Ordering::Relaxed);
        state
            .autolocate_fixes_written
            .store(4, std::sync::atomic::Ordering::Relaxed);
        state
            .db_read_failures
            .store(5, std::sync::atomic::Ordering::Relaxed);

        let response: serde_json::Value =
            serde_json::from_str(&handle_stats(&state).await).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["v"], 1);
        // Transitional dual-emit: both the new and deprecated fields carry the
        // same fallback 0 while the counter row is corrupt (task-0130).
        assert_eq!(response["wigle_lookups_today"], 0);
        assert_eq!(response["api_calls_today"], 0);
        // Transitional dual-emit (task-0268): both the new wigle_ field and the
        // deprecated api_ mirror carry the same corruption-denial count.
        assert_eq!(response["wigle_counter_corruption_denials"], 3);
        assert_eq!(response["api_counter_corruption_denials"], 3);
        assert_eq!(response["autolocate_fixes_written"], 4);
        assert_eq!(response["db_read_failures"], 5);
    }

    /// task-0254: a real read error in `db.stats()` silently zeroes the row
    /// counts (they would lie). Drop `aps` (which `stats()` counts first) so the
    /// read errors, then assert the fault is counted via db_read_failures AND
    /// the same response reflects it (the snapshot is taken after `stats()`).
    #[tokio::test]
    async fn handle_stats_counts_db_read_failure_when_stats_read_errors() {
        use std::sync::atomic::Ordering::Relaxed;

        let state = test_state();
        lock_db(&state)
            .execute_batch_for_test("DROP TABLE aps;")
            .unwrap();

        assert_eq!(state.db_read_failures.load(Relaxed), 0);
        let response: serde_json::Value =
            serde_json::from_str(&handle_stats(&state).await).unwrap();
        assert_eq!(
            state.db_read_failures.load(Relaxed),
            1,
            "stats() read error must be counted"
        );
        assert_eq!(
            response["db_read_failures"], 1,
            "the faulting response must reflect the increment (post-stats snapshot)"
        );
        assert_eq!(
            response["cached_aps"], 0,
            "row counts fall back to 0, not stale/lying data"
        );
    }

    /// task-0254: a real read error in `debug_statuses` silently mislabels every
    /// BSSID's DB status. Seed a scan so debug has an input BSSID, drop `aps`
    /// (read during the per-BSSID cache check) so the status read errors, then
    /// assert the fault is counted.
    #[tokio::test]
    async fn handle_debug_counts_db_read_failure_when_status_read_errors() {
        use std::sync::atomic::Ordering::Relaxed;

        let state = test_state();
        {
            let mut deb = state.debouncer.lock().await;
            let mut sample = crate::debounce::ScanSample::new();
            sample.insert(
                "aa:bb:cc:dd:ee:01".to_string(),
                crate::debounce::ScanEntry {
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    signal_dbm: -50,
                    ssid: None,
                    channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
                },
            );
            deb.push_scan(sample);
        }
        lock_db(&state)
            .execute_batch_for_test("DROP TABLE aps;")
            .unwrap();

        assert_eq!(state.db_read_failures.load(Relaxed), 0);
        let _ = handle_debug(&state).await;
        assert_eq!(
            state.db_read_failures.load(Relaxed),
            1,
            "debug_statuses read error must be counted"
        );
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
        let old_row = old_fix.clone();
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
            last.as_ref().expect("last_fix missing in memory").clone()
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
        assert_eq!(disk_row.at, memory_row.at);
        assert_eq!(disk_row.sources, memory_row.sources);
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
            provider_reported_lat: lat,
            provider_reported_lon: lon,
            encryption: None,
            frequency: None,
            city: None,
            country: None,
            is_mobile: false,
            source: crate::db::Source::Manual,
        };
        lock_db(state).upsert_ap(&ap).unwrap();
    }

    /// Build a real cache/scan fixture where every persisted-mobile and
    /// non-finite row is stronger than every usable fixed row. Scanner SSIDs
    /// deliberately hide or disagree with the persisted mobile SSIDs, so only
    /// the post-DB eligibility pass can reject those rows.
    fn cached_ineligible_rows_ahead_of_fixed(
        state: &Arc<DaemonState>,
        prefix: u8,
    ) -> crate::debounce::ScanSample {
        let mut sample = crate::debounce::ScanSample::new();

        for index in 0..11 {
            let bssid = format!("02:00:00:{prefix:02x}:00:{index:02x}");
            lock_db(state)
                .upsert_ap(&crate::db::ApInfo {
                    bssid: bssid.clone(),
                    ssid: Some(format!("MOVIA_PERSISTED_{index}")),
                    provider_reported_lat: 55.7390 + index as f64 / 100_000.0,
                    provider_reported_lon: 12.5500,
                    encryption: None,
                    frequency: None,
                    city: None,
                    country: None,
                    is_mobile: false,
                    source: crate::db::Source::Apple,
                })
                .unwrap();
            sample.insert(
                bssid,
                crate::debounce::ScanEntry {
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    signal_dbm: -10 - index,
                    // Exercise both scanner-hidden and currently nonmatching
                    // observations. Neither can reclassify the persisted row.
                    ssid: (index % 2 != 0).then(|| format!("ordinary-{index}")),
                    channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
                },
            );
        }

        for index in 0..11 {
            let bssid = format!("02:00:00:{prefix:02x}:01:{index:02x}");
            lock_db(state)
                .upsert_ap(&crate::db::ApInfo {
                    bssid: bssid.clone(),
                    ssid: Some(format!("poisoned-fixed-{index}")),
                    provider_reported_lat: f64::INFINITY,
                    provider_reported_lon: 12.56,
                    encryption: None,
                    frequency: None,
                    city: None,
                    country: None,
                    is_mobile: false,
                    source: crate::db::Source::Apple,
                })
                .unwrap();
            sample.insert(
                bssid,
                crate::debounce::ScanEntry {
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    signal_dbm: -30 - index,
                    ssid: Some(format!("poisoned-fixed-{index}")),
                    channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
                },
            );
        }

        for (index, (lat, lon)) in [(55.6760, 12.5680), (55.6761, 12.5682), (55.6759, 12.5681)]
            .into_iter()
            .enumerate()
        {
            // Universally-administered (U/L bit clear) so these are the only
            // trilateration-eligible rows: the MOVIA :00: rows are persisted
            // mobile and the :01: rows are non-finite, both excluded by earlier
            // guards; keeping THESE locally-administered would (correctly, per
            // task-0122) drop them from the solve too, defeating the fixture.
            let bssid = format!("b8:00:00:{prefix:02x}:02:{index:02x}");
            let ssid = format!("fixed-home-{index}");
            lock_db(state)
                .upsert_ap(&crate::db::ApInfo {
                    bssid: bssid.clone(),
                    ssid: Some(ssid.clone()),
                    provider_reported_lat: lat,
                    provider_reported_lon: lon,
                    encryption: None,
                    frequency: None,
                    city: None,
                    country: None,
                    is_mobile: false,
                    source: crate::db::Source::Apple,
                })
                .unwrap();
            sample.insert(
                bssid,
                crate::debounce::ScanEntry {
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    signal_dbm: -80 - index as i32,
                    ssid: Some(ssid),
                    channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
                },
            );
        }

        let db = lock_db(state);
        assert!(
            db.get_ap(&format!("02:00:00:{prefix:02x}:00:00"))
                .unwrap()
                .unwrap()
                .is_mobile,
            "fixture must persist mobile=true independently of current scanner SSID"
        );
        assert!(
            !db.get_ap(&format!("02:00:00:{prefix:02x}:01:00"))
                .unwrap()
                .unwrap()
                .is_mobile,
            "non-finite fixture must exercise finite eligibility, not mobile filtering"
        );
        sample
    }

    fn assert_three_fixed_sources(response: &str) {
        let json: serde_json::Value = serde_json::from_str(response).unwrap();
        assert_eq!(json["ok"], true, "locate failed: {response}");
        assert_eq!(json["cached"], 3, "only admitted cache rows count");
        assert_eq!(json["sources"], 3, "only fixed finite rows are sources");
        assert!((json["lat"].as_f64().unwrap() - 55.6760).abs() < 0.001);
        assert!((json["lon"].as_f64().unwrap() - 12.5681).abs() < 0.001);
    }

    async fn push_stable_scan(state: &Arc<DaemonState>, sample: crate::debounce::ScanSample) {
        let refresh_started_at = BootTime::now().expect("tests require Linux CLOCK_BOOTTIME");
        push_stable_scan_with_evidence(state, sample, Some(refresh_started_at)).await;
    }

    async fn push_stable_scan_with_evidence(
        state: &Arc<DaemonState>,
        sample: crate::debounce::ScanSample,
        confirmed_refresh_started_at: Option<BootTime>,
    ) {
        let mut debouncer = state.debouncer.lock().await;
        for _ in 0..state.args.debounce_threshold {
            debouncer.push_scan_with_refresh_evidence(sample.clone(), confirmed_refresh_started_at);
        }
    }

    fn add_cached_scan_ap(
        state: &Arc<DaemonState>,
        sample: &mut crate::debounce::ScanSample,
        bssid: &str,
        lat: f64,
        lon: f64,
        signal_dbm: i32,
        ssid: Option<&str>,
    ) {
        cache_ap(state, bssid, lat, lon);
        sample.insert(
            bssid.to_string(),
            crate::debounce::ScanEntry {
                rssi_source: crate::rssi::RssiSource::IwDbm,
                signal_dbm,
                ssid: ssid.map(str::to_owned),
                channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
            },
        );
    }

    fn previous_row(lat: f64, lon: f64, age: chrono::Duration) -> LastFix {
        LastFix {
            lat,
            lon,
            accuracy_m: 20.0,
            address: Some("previous address".to_string()),
            at: chrono::Utc::now() - age,
            sources: 7,
        }
    }

    fn fixed_autolocate_time() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-07-05T12:00:00.123456789+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    async fn seed_autolocate_cluster(state: &Arc<DaemonState>, prefix: u8, lat: f64, lon: f64) {
        let refresh_started_at = BootTime::now().expect("tests require Linux CLOCK_BOOTTIME");
        seed_autolocate_cluster_with_evidence(state, prefix, lat, lon, Some(refresh_started_at))
            .await;
    }

    async fn seed_autolocate_cluster_with_evidence(
        state: &Arc<DaemonState>,
        prefix: u8,
        lat: f64,
        lon: f64,
        confirmed_refresh_started_at: Option<BootTime>,
    ) {
        let mut sample = crate::debounce::ScanSample::new();
        for index in 0..3 {
            add_cached_scan_ap(
                state,
                &mut sample,
                &format!("b8:00:00:00:{prefix:02x}:{index:02x}"),
                lat,
                lon,
                -40 - index,
                Some("fixed-autolocate-test"),
            );
        }
        push_stable_scan_with_evidence(state, sample, confirmed_refresh_started_at).await;
    }

    async fn history_len(state: &Arc<DaemonState>) -> usize {
        state
            .db
            .get_fixes_in_range("0000", "9999")
            .await
            .unwrap()
            .len()
    }

    async fn history_rows(state: &Arc<DaemonState>) -> Vec<crate::db::Fix> {
        state.db.get_fixes_in_range("0000", "9999").await.unwrap()
    }

    /// Assert a persisted quality speed equals the speed reported in a locate
    /// JSON response. Both are the SAME in-memory f64 (`decision.quality`), but
    /// the DB round-trip is bit-exact while reading the value back out of the
    /// JSON here goes through serde_json's default float parser, which is not
    /// guaranteed correctly-rounded (`float_roundtrip` is off) and can land 1
    /// ULP away. A relative tolerance (~1e-9, ~1e6x the ULP, far below any
    /// meaningful speed difference) expresses the real "history == response"
    /// invariant without the ~8% exact-equality flake this used to have.
    fn assert_speed_matches(persisted: Option<f64>, reported: Option<f64>) {
        match (persisted, reported) {
            (Some(persisted), Some(reported)) => assert!(
                (persisted - reported).abs() <= 1e-9 * persisted.abs().max(1.0),
                "persisted speed {persisted} disagrees with reported {reported}"
            ),
            (persisted, reported) => assert_eq!(persisted, reported),
        }
    }

    fn scan_snapshot_with_evidence(
        confirmed_refresh_started_at: Option<BootTime>,
    ) -> TimestampedScan {
        TimestampedScan {
            sample: crate::debounce::ScanSample::new(),
            received_at: std::time::Instant::now(),
            // Deliberately unrelated to BootTime. Wall-clock movement must not
            // affect autonomous freshness.
            wall_clock: fixed_autolocate_time() + chrono::TimeDelta::days(365),
            confirmed_refresh_started_at,
        }
    }

    #[test]
    fn autolocate_scan_freshness_exact_boundary_and_one_nanosecond_over() {
        let refresh = BootTime::from_duration(Duration::from_secs(100));
        let snapshot = scan_snapshot_with_evidence(Some(refresh));
        let exact = BootTime::from_duration(Duration::from_secs(280));
        let over = BootTime::from_duration(Duration::from_secs(280) + Duration::from_nanos(1));

        assert_eq!(
            classify_autolocate_scan(Some(&snapshot), 180, || Ok(exact)).unwrap(),
            AutolocateScanFreshness::Fresh,
            "maximum age equality must remain fresh"
        );
        assert_eq!(
            classify_autolocate_scan(Some(&snapshot), 180, || Ok(over)).unwrap(),
            AutolocateScanFreshness::Stale {
                age: Duration::from_secs(180) + Duration::from_nanos(1)
            },
            "one nanosecond over the maximum must be stale"
        );
    }

    #[test]
    fn absent_unconfirmed_and_zero_override_do_not_read_boot_clock_unnecessarily() {
        assert_eq!(
            classify_autolocate_scan(None, 180, || panic!("absent must not read clock")).unwrap(),
            AutolocateScanFreshness::Absent
        );
        assert_eq!(
            classify_autolocate_scan(None, 0, || panic!("absent must not read clock")).unwrap(),
            AutolocateScanFreshness::Absent,
            "zero never manufactures a missing snapshot"
        );

        let unconfirmed = scan_snapshot_with_evidence(None);
        assert_eq!(
            classify_autolocate_scan(Some(&unconfirmed), 180, || {
                panic!("unconfirmed must reject before reading clock")
            })
            .unwrap(),
            AutolocateScanFreshness::Unconfirmed
        );
        assert_eq!(
            classify_autolocate_scan(Some(&unconfirmed), 0, || {
                panic!("legacy override must not read clock")
            })
            .unwrap(),
            AutolocateScanFreshness::Fresh,
            "zero explicitly accepts a present unconfirmed snapshot"
        );
    }

    #[test]
    fn boot_time_not_received_or_wall_time_controls_suspend_inclusive_age() {
        let refresh = BootTime::from_duration(Duration::from_secs(1));
        let snapshot = scan_snapshot_with_evidence(Some(refresh));
        // received_at was just created and wall_clock is a year ahead, but a
        // 181-second CLOCK_BOOTTIME delta (including a hypothetical suspend)
        // still rejects the snapshot.
        assert_eq!(
            classify_autolocate_scan(Some(&snapshot), 180, || {
                Ok(BootTime::from_duration(Duration::from_secs(182)))
            })
            .unwrap(),
            AutolocateScanFreshness::Stale {
                age: Duration::from_secs(181)
            }
        );
    }

    #[test]
    fn clock_read_and_future_evidence_fail_closed_with_actionable_warning() {
        // task-0255: shared capture sink (installs the permissive global,
        // task-0248, internally). This site filters WARN and maps to message.
        let refresh = BootTime::from_duration(Duration::from_secs(20));
        let snapshot = scan_snapshot_with_evidence(Some(refresh));

        let (outcome, events) = crate::test_support::capture(|| {
            admit_autolocate_scan(Some(&snapshot), 180, || {
                Err(std::io::Error::other("injected boot clock failure"))
            })
        });
        assert_eq!(outcome, Err(AutolocateOutcome::ScanClockFailure));
        let warnings: Vec<String> = events
            .iter()
            .filter(|(level, _, _)| *level == tracing::Level::WARN)
            .map(|(_, _, message)| message.clone())
            .collect();
        assert!(
            warnings.iter().any(|message| {
                message.contains("failed closed before cache access")
                    && message.contains("failed to read CLOCK_BOOTTIME")
                    && message.contains("injected boot clock failure")
            }),
            "missing actionable clock warning: {warnings:?}"
        );

        let (outcome, events) = crate::test_support::capture(|| {
            admit_autolocate_scan(Some(&snapshot), 180, || {
                Ok(BootTime::from_duration(Duration::from_secs(19)))
            })
        });
        assert_eq!(outcome, Err(AutolocateOutcome::ScanClockFailure));
        let warnings: Vec<String> = events
            .iter()
            .filter(|(level, _, _)| *level == tracing::Level::WARN)
            .map(|(_, _, message)| message.clone())
            .collect();
        assert!(
            warnings.iter().any(|message| {
                message.contains("failed closed before cache access")
                    && message.contains("is ahead of current CLOCK_BOOTTIME")
                    && message.contains("BootTime(20s)")
                    && message.contains("BootTime(19s)")
            }),
            "missing actionable future-evidence warning: {warnings:?}"
        );
    }

    #[test]
    fn autolocate_dedup_boundaries_are_strict_and_high_resolution() {
        let just_inside =
            chrono::TimeDelta::seconds(119) + chrono::TimeDelta::nanoseconds(999_999_999);
        assert!(autolocate_metrics_are_duplicate(
            99.999_999,
            just_inside,
            100,
            120
        ));
        assert!(
            !autolocate_metrics_are_duplicate(100.0, just_inside, 100, 120),
            "distance equality must write"
        );
        assert!(
            !autolocate_metrics_are_duplicate(99.0, chrono::TimeDelta::seconds(120), 100, 120),
            "age equality must write"
        );
    }

    #[tokio::test]
    async fn autolocate_first_fix_commits_history_memory_counter_and_cached_address() {
        let mut state = test_state();
        Arc::get_mut(&mut state)
            .expect("state must be uniquely owned")
            .args
            .address_approx = true;
        let location = (55.6761, 12.5683);
        seed_autolocate_cluster(&state, 0x70, location.0, location.1).await;
        state.address_cache.lock().unwrap().insert(
            location.0,
            location.1,
            "cached street".to_string(),
        );

        let now = fixed_autolocate_time();
        let outcome = run_autolocate_pass_at(&state, state.privacy.ticket(), now).await;
        assert_eq!(outcome, AutolocateOutcome::Written);
        assert_eq!(history_len(&state).await, 1);
        let rows = history_rows(&state).await;
        let quality = rows[0]
            .quality
            .as_ref()
            .expect("new autonomous rows must have quality");
        assert_eq!(quality.confidence(), Confidence::High);
        assert_eq!(quality.implied_speed_kmh(), None);
        assert_eq!(quality.last_fix_age_secs(), None);
        assert_eq!(quality.inlier_ratio(), Some(1.0));
        assert_eq!(quality.suspect_reason(), None);
        assert_eq!(
            state
                .autolocate_fixes_written
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        let persisted = state.db.get_last_fix().await.unwrap().unwrap();
        assert_eq!(persisted.at, now);
        assert_eq!(persisted.address.as_deref(), Some("cached street"));
        let memory = state.last_fix.lock().await;
        assert_eq!(memory.as_ref().unwrap().at, now);
        assert_eq!(
            memory.as_ref().unwrap().address.as_deref(),
            Some("cached street")
        );
        assert_eq!(
            state.geocode.snapshot().pending,
            0,
            "autolocate must not schedule Nominatim"
        );
    }

    #[tokio::test]
    async fn task0245_autolocate_suspect_current_persists_exact_decision_quality() {
        const CURRENT: (f64, f64) = (55.6761, 12.5683);
        const PREVIOUS: (f64, f64) = (-12.8940, -38.2920);
        let state = test_state();
        let now = fixed_autolocate_time();
        let previous = LastFix {
            lat: PREVIOUS.0,
            lon: PREVIOUS.1,
            accuracy_m: 20.0,
            address: None,
            at: now - chrono::TimeDelta::minutes(5),
            sources: 5,
        };
        state.db.set_last_fix(previous.clone()).await.unwrap();
        *state.last_fix.lock().await = Some(previous);

        let mut sample = crate::debounce::ScanSample::new();
        for index in 0..4 {
            add_cached_scan_ap(
                &state,
                &mut sample,
                &format!("b8:00:00:00:84:{index:02x}"),
                CURRENT.0,
                CURRENT.1,
                -40 - index,
                Some("fixed-current"),
            );
        }
        add_cached_scan_ap(
            &state,
            &mut sample,
            "b8:00:00:00:84:ff",
            PREVIOUS.0,
            PREVIOUS.1,
            -80,
            Some("fixed-previous"),
        );
        push_stable_scan(&state, sample).await;

        assert_eq!(
            run_autolocate_pass_at(&state, state.privacy.ticket(), now).await,
            AutolocateOutcome::Written
        );
        let rows = history_rows(&state).await;
        assert_eq!(rows.len(), 1);
        let quality = rows[0].quality.as_ref().unwrap();
        assert_eq!(quality.confidence(), Confidence::Suspect);
        assert_eq!(quality.last_fix_age_secs(), Some(300));
        assert_eq!(quality.inlier_ratio(), Some(0.8));
        assert!(quality.implied_speed_kmh().unwrap() > 250.0);
        let reason = quality.suspect_reason().unwrap();
        assert!(
            reason.contains("tie-break favored current_trilat"),
            "{reason}"
        );
        assert!(
            reason.contains("score current_trilat 0.80 vs last_fix 0.20"),
            "{reason}"
        );
    }

    #[tokio::test]
    async fn autolocate_identical_second_pass_deduplicates() {
        let state = test_state();
        seed_autolocate_cluster(&state, 0x71, 55.6761, 12.5683).await;
        let bssids: Vec<String> = (0..3)
            .map(|index| format!("b8:00:00:00:71:{index:02x}"))
            .collect();
        const OLD_LAST_SEEN: &str = "2000-01-01T00:00:00+00:00";
        state
            .db
            .set_cached_last_seen_for_test(bssids.clone(), OLD_LAST_SEEN.to_string())
            .await
            .unwrap();
        let now = fixed_autolocate_time();
        assert_eq!(
            run_autolocate_pass_at(&state, state.privacy.ticket(), now).await,
            AutolocateOutcome::Written
        );
        assert_eq!(
            run_autolocate_pass_at(
                &state,
                state.privacy.ticket(),
                now + chrono::TimeDelta::nanoseconds(1),
            )
            .await,
            AutolocateOutcome::Deduplicated
        );
        assert_eq!(history_len(&state).await, 1);
        let rows = history_rows(&state).await;
        let quality = rows[0]
            .quality
            .as_ref()
            .expect("deduplicated pass must retain first observation quality");
        assert_eq!(quality.confidence(), Confidence::High);
        assert_eq!(quality.implied_speed_kmh(), None);
        assert_eq!(quality.last_fix_age_secs(), None);
        assert_eq!(quality.inlier_ratio(), Some(1.0));
        assert!(state
            .db
            .cached_last_seen_for_test(bssids)
            .await
            .unwrap()
            .iter()
            .all(|last_seen| last_seen == OLD_LAST_SEEN));
        assert_eq!(
            state
                .autolocate_fixes_written
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn autolocate_age_equality_writes_the_retained_snapshot_again() {
        let state = test_state();
        seed_autolocate_cluster(&state, 0x72, 55.6761, 12.5683).await;
        let now = fixed_autolocate_time();
        assert_eq!(
            run_autolocate_pass_at(&state, state.privacy.ticket(), now).await,
            AutolocateOutcome::Written
        );
        assert_eq!(
            run_autolocate_pass_at(
                &state,
                state.privacy.ticket(),
                now + chrono::TimeDelta::seconds(state.args.autolocate_interval_secs as i64),
            )
            .await,
            AutolocateOutcome::Written,
            "age equality must not deduplicate"
        );
        assert_eq!(history_len(&state).await, 2);
        assert_eq!(
            state
                .autolocate_fixes_written
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );
    }

    #[tokio::test]
    async fn autolocate_future_last_fix_uses_zero_age_and_deduplicates() {
        let state = test_state();
        let location = (55.6761, 12.5683);
        seed_autolocate_cluster(&state, 0x73, location.0, location.1).await;
        let now = fixed_autolocate_time();
        state
            .db
            .set_last_fix(LastFix {
                lat: location.0,
                lon: location.1,
                accuracy_m: 5.0,
                address: None,
                at: now + chrono::TimeDelta::seconds(1),
                sources: 3,
            })
            .await
            .unwrap();

        assert_eq!(
            run_autolocate_pass_at(&state, state.privacy.ticket(), now).await,
            AutolocateOutcome::Deduplicated
        );
        assert_eq!(history_len(&state).await, 0);
        assert_eq!(
            state
                .autolocate_fixes_written
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn autolocate_clock_backward_appends_history_without_regressing_last_fix() {
        let state = test_state();
        let now = fixed_autolocate_time();
        let future = now + chrono::TimeDelta::seconds(1);
        let previous = LastFix {
            lat: 55.6761,
            lon: 12.5683,
            accuracy_m: 5.0,
            address: Some("future authority".to_string()),
            at: future,
            sources: 3,
        };
        state.db.set_last_fix(previous.clone()).await.unwrap();
        *state.last_fix.lock().await = Some(previous);
        // Far enough to bypass distance dedup. A non-positive elapsed time has
        // zero implied speed, so TASK-0094 accepts the current observation;
        // SQLite's monotonic guard must still retain the future last_fix.
        seed_autolocate_cluster(&state, 0x7a, -12.8940, -38.2920).await;

        assert_eq!(
            run_autolocate_pass_at(&state, state.privacy.ticket(), now).await,
            AutolocateOutcome::Written
        );
        assert_eq!(history_len(&state).await, 1);
        assert_eq!(
            state
                .autolocate_fixes_written
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        let rows = history_rows(&state).await;
        let quality = rows[0].quality.as_ref().unwrap();
        assert_eq!(quality.confidence(), Confidence::High);
        assert_eq!(quality.implied_speed_kmh(), Some(0.0));
        assert_eq!(quality.last_fix_age_secs(), Some(0));
        assert_eq!(quality.inlier_ratio(), Some(1.0));
        assert_eq!(
            state
                .db
                .get_last_fix()
                .await
                .unwrap()
                .unwrap()
                .at
                .to_rfc3339(),
            future.to_rfc3339()
        );
        assert_eq!(state.last_fix.lock().await.as_ref().unwrap().at, future);
    }

    #[tokio::test]
    async fn autolocate_skip_order_has_no_provider_pending_or_geocode_effects() {
        let state = test_state();
        let bssid = "b8:00:00:00:74:00";
        let mut sample = crate::debounce::ScanSample::new();
        sample.insert(
            bssid.to_string(),
            crate::debounce::ScanEntry {
                rssi_source: crate::rssi::RssiSource::IwDbm,
                signal_dbm: -45,
                ssid: Some("uncached-fixed".to_string()),
                channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
            },
        );
        let refresh_started_at = BootTime::now().expect("tests require Linux CLOCK_BOOTTIME");
        state
            .debouncer
            .lock()
            .await
            .push_scan_with_refresh_evidence(sample.clone(), Some(refresh_started_at));
        let now = fixed_autolocate_time();
        assert_eq!(
            run_autolocate_pass_at(&state, state.privacy.ticket(), now).await,
            AutolocateOutcome::NoStableCandidates
        );
        {
            let mut debouncer = state.debouncer.lock().await;
            for _ in 1..state.args.debounce_threshold {
                debouncer.push_scan_with_refresh_evidence(sample.clone(), Some(refresh_started_at));
            }
        }
        assert_eq!(
            run_autolocate_pass_at(&state, state.privacy.ticket(), now).await,
            AutolocateOutcome::NoCachedPositions
        );
        assert_eq!(lock_db(&state).pending_ap_count().unwrap(), 0);
        assert!(state.inflight.lock().unwrap().is_empty());
        assert_eq!(state.tasks.lock().unwrap().len(), 0);
        assert_eq!(state.geocode.snapshot().pending, 0);
        assert_eq!(history_len(&state).await, 0);
        assert_eq!(
            state
                .autolocate_fixes_written
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stale_scan_rejects_before_db_cache_and_every_autonomous_side_effect() {
        let state = test_state();
        let wall_now = fixed_autolocate_time();
        let previous = LastFix {
            lat: 55.6761,
            lon: 12.5683,
            accuracy_m: 5.0,
            address: Some("must remain unchanged".to_string()),
            at: wall_now - chrono::TimeDelta::minutes(1),
            sources: 3,
        };
        state.db.set_last_fix(previous.clone()).await.unwrap();
        *state.last_fix.lock().await = Some(previous.clone());

        let refresh = BootTime::from_duration(Duration::from_secs(100));
        seed_autolocate_cluster_with_evidence(&state, 0x7c, 55.6761, 12.5683, Some(refresh)).await;
        let commits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        state
            .db
            .install_writer_commit_counter(Arc::clone(&commits))
            .await
            .unwrap();

        // If freshness admission reaches even the read-only cache actor, this
        // pass blocks here and the timeout fails. The reader trap is stronger
        // than checking AP last_seen, because autonomous cache reads are
        // intentionally non-touching even when they do execute.
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker_db = state.db.clone();
        let mut blocker =
            tokio::spawn(async move { blocker_db.hold_reader_until(entered_tx, release_rx).await });
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("reader blocker did not start");

        let outcome = tokio::time::timeout(
            Duration::from_millis(250),
            run_autolocate_pass_at_boot_time(
                &state,
                state.privacy.ticket(),
                wall_now,
                BootTime::from_duration(Duration::from_secs(280) + Duration::from_nanos(1)),
            ),
        )
        .await;
        let _ = release_tx.send(());
        tokio::time::timeout(Duration::from_secs(2), &mut blocker)
            .await
            .expect("reader blocker did not stop")
            .expect("reader blocker task panicked")
            .expect("reader blocker failed");
        assert_eq!(outcome.unwrap(), AutolocateOutcome::StaleScanSnapshot);

        assert_eq!(history_len(&state).await, 0);
        assert_eq!(
            state
                .db
                .get_last_fix()
                .await
                .unwrap()
                .unwrap()
                .at
                .to_rfc3339(),
            previous.at.to_rfc3339()
        );
        assert_eq!(
            state.last_fix.lock().await.as_ref().unwrap().at,
            previous.at
        );
        assert_eq!(
            state
                .autolocate_fixes_written
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(commits.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(lock_db(&state).pending_ap_count().unwrap(), 0);
        assert!(state.inflight.lock().unwrap().is_empty());
        assert_eq!(state.tasks.lock().unwrap().len(), 0);
        assert_eq!(state.geocode.snapshot().pending, 0);
    }

    #[tokio::test]
    async fn repeated_scanner_errors_keep_snapshot_stale_until_confirmed_recovery() {
        let state = test_state();
        let wall_now = fixed_autolocate_time();
        let old_refresh = BootTime::from_duration(Duration::from_secs(100));
        seed_autolocate_cluster_with_evidence(&state, 0x7d, 55.6761, 12.5683, Some(old_refresh))
            .await;
        let stale_now = BootTime::from_duration(Duration::from_secs(281));

        // Total scanner errors publish no batch, so repeated attempts leave
        // the exact retained evidence untouched and cannot rejuvenate it.
        for _ in 0..2 {
            assert_eq!(
                run_autolocate_pass_at_boot_time(
                    &state,
                    state.privacy.ticket(),
                    wall_now,
                    stale_now,
                )
                .await,
                AutolocateOutcome::StaleScanSnapshot
            );
        }
        assert_eq!(history_len(&state).await, 0);

        // One new confirmed radio refresh atomically replaces the latest
        // sample/evidence. The pre-existing ring already establishes
        // stability, so the next pass can recover immediately.
        let sample = state
            .debouncer
            .lock()
            .await
            .latest_scan()
            .expect("fixture scan missing")
            .clone();
        state
            .debouncer
            .lock()
            .await
            .push_scan_with_refresh_evidence(sample, Some(stale_now));
        assert_eq!(
            run_autolocate_pass_at_boot_time(&state, state.privacy.ticket(), wall_now, stale_now,)
                .await,
            AutolocateOutcome::Written
        );
        assert_eq!(history_len(&state).await, 1);
    }

    #[tokio::test]
    async fn unconfirmed_scan_rejects_by_default_but_zero_restores_legacy_behavior() {
        let state = test_state();
        seed_autolocate_cluster_with_evidence(&state, 0x7e, 55.6761, 12.5683, None).await;
        assert_eq!(
            run_autolocate_pass_at_boot_time(
                &state,
                state.privacy.ticket(),
                fixed_autolocate_time(),
                BootTime::from_duration(Duration::from_secs(999)),
            )
            .await,
            AutolocateOutcome::UnconfirmedScanSnapshot
        );
        assert_eq!(history_len(&state).await, 0);

        let mut legacy = test_state();
        Arc::get_mut(&mut legacy)
            .expect("state must be uniquely owned")
            .args
            .autolocate_max_scan_age_secs = 0;
        seed_autolocate_cluster_with_evidence(&legacy, 0x7f, 55.6761, 12.5683, None).await;
        assert_eq!(
            run_autolocate_pass_at_boot_time(
                &legacy,
                legacy.privacy.ticket(),
                fixed_autolocate_time(),
                BootTime::from_duration(Duration::from_secs(999)),
            )
            .await,
            AutolocateOutcome::Written
        );
        assert_eq!(history_len(&legacy).await, 1);
    }

    #[tokio::test]
    async fn interactive_locate_keeps_unconfirmed_and_stale_snapshot_compatibility() {
        let unconfirmed = test_state();
        seed_autolocate_cluster_with_evidence(&unconfirmed, 0x80, 55.6761, 12.5683, None).await;
        let response: serde_json::Value =
            serde_json::from_str(&dispatch_command(&Request::Locate, &unconfirmed).await).unwrap();
        assert_eq!(response["ok"], true);

        let stale = test_state();
        seed_autolocate_cluster_with_evidence(
            &stale,
            0x81,
            55.6761,
            12.5683,
            Some(BootTime::from_duration(Duration::ZERO)),
        )
        .await;
        let response: serde_json::Value =
            serde_json::from_str(&dispatch_command(&Request::Locate, &stale).await).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(history_len(&unconfirmed).await, 1);
        assert_eq!(history_len(&stale).await, 1);
    }

    #[tokio::test]
    async fn autolocate_db_failure_leaves_memory_and_counter_unchanged() {
        let state = test_state();
        let now = fixed_autolocate_time();
        let previous = LastFix {
            lat: 55.6761,
            lon: 12.5683,
            accuracy_m: 5.0,
            address: Some("unchanged on failure".to_string()),
            at: now + chrono::TimeDelta::seconds(1),
            sources: 3,
        };
        state.db.set_last_fix(previous.clone()).await.unwrap();
        *state.last_fix.lock().await = Some(previous.clone());
        seed_autolocate_cluster(&state, 0x75, -12.8940, -38.2920).await;
        lock_db(&state)
            .execute_batch_for_test(
                "CREATE TRIGGER fail_autolocate_fix
                 BEFORE INSERT ON fixes
                 BEGIN SELECT RAISE(FAIL, 'forced autolocate failure'); END;",
            )
            .unwrap();

        assert_eq!(
            run_autolocate_pass_at(&state, state.privacy.ticket(), now).await,
            AutolocateOutcome::Failed
        );
        assert_eq!(
            state.last_fix.lock().await.as_ref().unwrap().at,
            previous.at
        );
        assert_eq!(
            state
                .db
                .get_last_fix()
                .await
                .unwrap()
                .unwrap()
                .at
                .to_rfc3339(),
            previous.at.to_rfc3339()
        );
        assert_eq!(history_len(&state).await, 0);
        assert_eq!(
            state
                .autolocate_fixes_written
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            state
                .db_write_failures
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn autolocate_last_fix_read_failure_warns_root_cause_and_interactive_json_stays_actionable(
    ) {
        use tracing::instrument::WithSubscriber as _;
        crate::test_support::install_permissive_global_test_subscriber(); // task-0248: deterministic scoped capture

        let state = test_state();
        seed_autolocate_cluster(&state, 0x7b, 55.6761, 12.5683).await;
        lock_db(&state)
            .execute_batch_for_test("DROP TABLE last_fix;")
            .unwrap();

        // task-0255: raw shared CaptureSink (async site attaches the sink to
        // the future via WithSubscriber, so the scoped `capture` helper does
        // not fit). The permissive global is installed above (task-0248/0251).
        let captured = crate::test_support::CaptureSink::default();
        let handle = captured.clone();
        let outcome =
            run_autolocate_pass_at(&state, state.privacy.ticket(), fixed_autolocate_time())
                .with_subscriber(captured)
                .await;
        assert_eq!(outcome, AutolocateOutcome::Failed);
        let warnings: Vec<String> = handle
            .events()
            .iter()
            .filter(|(level, _, _)| *level == tracing::Level::WARN)
            .map(|(_, _, message)| message.clone())
            .collect();
        assert!(
            warnings.iter().any(|message| {
                message.contains("autolocate failed to read persisted last_fix")
                    && message.contains("no such table: last_fix")
            }),
            "missing actionable root cause in warnings: {warnings:?}"
        );

        let interactive = dispatch_command(&Request::Locate, &state).await;
        let interactive: serde_json::Value = serde_json::from_str(&interactive).unwrap();
        assert_eq!(interactive["ok"], false);
        assert!(interactive["error"].as_str().is_some_and(
            |error| error.contains("failed to read last_fix: no such table: last_fix")
        ));
    }

    /// task-0134 review hardening: the sibling of the read-failure test above.
    /// A *corrupt* persisted last_fix (present row, unparseable timestamp) is a
    /// `CorruptLastFix` — a DISTINCT category from a read failure — so the
    /// locate path must IGNORE it and still answer with a fresh position, not
    /// hard-abort. This pins the `get_last_fix` -> `CorruptLastFix` -> downcast
    /// arm-ordering in handle_locate; if that ever regresses (e.g. a
    /// `.context(..)` wrap defeats the downcast), a corrupt row would silently
    /// become a fatal `failed to read last_fix` and this test would fail.
    #[tokio::test]
    async fn locate_with_corrupt_persisted_last_fix_still_answers() {
        use tracing::instrument::WithSubscriber as _;
        crate::test_support::install_permissive_global_test_subscriber(); // task-0248/0251

        let state = test_state();
        // Cached APs + a debounced stable scan so the interactive locate can
        // trilaterate a fresh position (mirrors the cold-boot locate test).
        let mut sample = crate::debounce::ScanSample::new();
        for (index, (lat, lon)) in [(55.6760, 12.5680), (55.6761, 12.5682), (55.6759, 12.5681)]
            .into_iter()
            .enumerate()
        {
            add_cached_scan_ap(
                &state,
                &mut sample,
                &format!("b8:00:00:00:3c:{index:02x}"),
                lat,
                lon,
                -50 - index as i32,
                Some("fixed-home"),
            );
        }
        push_stable_scan(&state, sample).await;

        // Persist a VALID last_fix, then corrupt its timestamp IN PLACE. A
        // string round-trips through the TEXT column (unlike NaN through a
        // REAL), so the read path parses and rejects it as `CorruptLastFix`.
        state
            .db
            .set_last_fix(LastFix {
                lat: 55.6761,
                lon: 12.5683,
                accuracy_m: 10.0,
                address: None,
                at: fixed_autolocate_time(),
                sources: 3,
            })
            .await
            .unwrap();
        lock_db(&state)
            .execute_batch_for_test(
                "UPDATE last_fix SET at_rfc3339 = 'not-a-timestamp' WHERE id = 1;",
            )
            .unwrap();

        let captured = crate::test_support::CaptureSink::default();
        let handle = captured.clone();
        let response = dispatch_command(&Request::Locate, &state)
            .with_subscriber(captured)
            .await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();

        // (1) The corrupt row is ignored, not fatal: locate still answers.
        assert_eq!(
            json["ok"], true,
            "corrupt persisted last_fix must be ignored, not abort locate: {response}"
        );
        assert!(
            json["lat"].as_f64().is_some() && json["lon"].as_f64().is_some(),
            "expected a fresh trilaterated position: {response}"
        );

        // (2) The ignore path is observable via the scoped warn capture.
        let warnings: Vec<String> = handle
            .events()
            .iter()
            .filter(|(level, _, _)| *level == tracing::Level::WARN)
            .map(|(_, _, message)| message.clone())
            .collect();
        assert!(
            warnings.iter().any(
                |message| message.contains("ignoring invalid persisted last_fix during locate")
            ),
            "expected the ignore-and-continue warn, got: {warnings:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_autolocate_passes_serialize_without_duplicate_history() {
        let state = test_state();
        seed_autolocate_cluster(&state, 0x76, 55.6761, 12.5683).await;
        let now = fixed_autolocate_time();
        let first_state = Arc::clone(&state);
        let second_state = Arc::clone(&state);
        let (first, second) = tokio::join!(
            async move { run_autolocate_pass_at(&first_state, first_state.privacy.ticket(), now).await },
            async move {
                run_autolocate_pass_at(&second_state, second_state.privacy.ticket(), now).await
            }
        );
        assert!(
            matches!(
                (first, second),
                (AutolocateOutcome::Written, AutolocateOutcome::Deduplicated)
                    | (AutolocateOutcome::Deduplicated, AutolocateOutcome::Written)
            ),
            "unexpected outcomes: {first:?}, {second:?}"
        );
        assert_eq!(history_len(&state).await, 1);
        assert_eq!(
            state
                .autolocate_fixes_written
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn wipe_clears_scan_evidence_and_recovery_requires_new_confirmed_scan() {
        let state = test_state();
        seed_autolocate_cluster(&state, 0x77, 55.6761, 12.5683).await;
        let stale = state.privacy.ticket();
        let response = dispatch_command(
            &Request::Wipe {
                confirm: Some(WIPE_CONFIRMATION.to_string()),
            },
            &state,
        )
        .await;
        drop(response);
        assert_eq!(
            run_autolocate_pass_at(&state, stale, fixed_autolocate_time()).await,
            AutolocateOutcome::Invalidated
        );
        assert_eq!(
            run_autolocate_pass_at(&state, state.privacy.ticket(), fixed_autolocate_time(),).await,
            AutolocateOutcome::NoScanSnapshot,
            "fresh post-wipe ticket must see the atomically cleared sample/evidence"
        );
        assert_eq!(history_len(&state).await, 0);

        seed_autolocate_cluster(&state, 0x78, 55.6761, 12.5683).await;
        assert_eq!(
            run_autolocate_pass_at(&state, state.privacy.ticket(), fixed_autolocate_time(),).await,
            AutolocateOutcome::Written
        );
        assert_eq!(history_len(&state).await, 1);
    }

    #[tokio::test]
    async fn autolocate_task0094_previous_winner_writes_nothing() {
        const PREVIOUS: (f64, f64) = (55.7047, 12.5900);
        const BRAZIL: (f64, f64) = (-12.8940, -38.2920);
        const OUTLIER: &str = "b8:00:00:00:79:ff";
        let mut state = test_state();
        Arc::get_mut(&mut state)
            .expect("state must be uniquely owned")
            .args
            .top_n = 1;
        let now = fixed_autolocate_time();
        state
            .db
            .set_last_fix(LastFix {
                lat: PREVIOUS.0,
                lon: PREVIOUS.1,
                accuracy_m: 20.0,
                address: Some("previous address".to_string()),
                at: now - chrono::TimeDelta::minutes(5),
                sources: 7,
            })
            .await
            .unwrap();
        let mut sample = crate::debounce::ScanSample::new();
        add_cached_scan_ap(
            &state,
            &mut sample,
            OUTLIER,
            BRAZIL.0,
            BRAZIL.1,
            -20,
            Some("strong-outlier"),
        );
        for index in 0..4 {
            add_cached_scan_ap(
                &state,
                &mut sample,
                &format!("b8:00:00:00:79:{index:02x}"),
                PREVIOUS.0 + index as f64 / 100_000.0,
                PREVIOUS.1 + index as f64 / 100_000.0,
                -60 - index,
                Some("near-previous"),
            );
        }
        // Only the strong Brazil outlier is stable enough to enter the solver.
        // The four currently visible but unstable cached APs near PREVIOUS
        // still belong to TASK-0094's consensus denominator. Looking up only
        // stable BSSIDs changes this decision to current and must fail here.
        assert!(state.args.debounce_threshold > 1);
        let mut outlier_only = crate::debounce::ScanSample::new();
        outlier_only.insert(OUTLIER.to_string(), sample[OUTLIER].clone());
        {
            let mut debouncer = state.debouncer.lock().await;
            let refresh_started_at = BootTime::now().expect("tests require Linux CLOCK_BOOTTIME");
            for _ in 1..state.args.debounce_threshold {
                debouncer.push_scan_with_refresh_evidence(
                    outlier_only.clone(),
                    Some(refresh_started_at),
                );
            }
            debouncer.push_scan_with_refresh_evidence(sample, Some(refresh_started_at));
            assert_eq!(debouncer.latest_scan().unwrap().len(), 5);
            assert_eq!(debouncer.stable_bssids(), [OUTLIER.to_string()].into());
        }

        assert_eq!(
            run_autolocate_pass_at(&state, state.privacy.ticket(), now).await,
            AutolocateOutcome::LastFixPreferred
        );
        assert_eq!(history_len(&state).await, 0);
        assert_eq!(
            state
                .autolocate_fixes_written
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            state
                .db
                .get_last_fix()
                .await
                .unwrap()
                .unwrap()
                .at
                .to_rfc3339(),
            (now - chrono::TimeDelta::minutes(5)).to_rfc3339()
        );
    }

    #[test]
    fn confidence_thresholds_and_display_are_pinned() {
        for (score, expected) in [
            (Some(1.0), Confidence::High),
            (Some(0.7), Confidence::High),
            (Some(0.699_999), Confidence::Medium),
            (Some(0.4), Confidence::Medium),
            (Some(0.399_999), Confidence::Low),
            (None, Confidence::Low),
        ] {
            assert_eq!(confidence_from_score(score), expected);
        }
        assert_eq!(Confidence::High.to_string(), "High");
        assert_eq!(Confidence::Medium.to_string(), "Medium");
        assert_eq!(Confidence::Low.to_string(), "Low");
        assert_eq!(Confidence::Suspect.to_string(), "Suspect");
        assert_eq!(
            serde_json::to_string(&Confidence::Suspect).unwrap(),
            "\"suspect\""
        );
    }

    #[test]
    fn speed_limit_equality_is_not_suspect_and_undefined_tie_favors_current() {
        let now = chrono::Utc::now();
        let previous = LastFix {
            lat: 55.6050,
            lon: 13.0038,
            accuracy_m: 10.0,
            address: None,
            at: now - chrono::Duration::hours(1),
            sources: 3,
        };
        let current = (55.6761, 12.5683);
        let exact_speed =
            crate::geo::haversine_kmh(previous.lat, previous.lon, current.0, current.1, 3600.0);
        let at_limit = evaluate_plausibility(
            Some(&previous),
            current,
            &[current, current, current],
            now,
            exact_speed,
        );
        assert_eq!(at_limit.winner, PlausibilityWinner::CurrentTrilat);
        assert_eq!(at_limit.quality.confidence(), Confidence::High);
        assert!(at_limit.quality.suspect_reason().is_none());

        let undefined_tie = evaluate_plausibility(
            Some(&previous),
            current,
            &[current, current],
            now,
            exact_speed - 0.001,
        );
        assert_eq!(undefined_tie.winner, PlausibilityWinner::CurrentTrilat);
        assert_eq!(undefined_tie.quality.confidence(), Confidence::Suspect);
        assert_eq!(undefined_tie.quality.inlier_ratio(), None);
        let reason = undefined_tie.quality.suspect_reason().unwrap();
        assert!(reason.contains("favored current_trilat"), "{reason}");
        assert!(
            reason.contains("score current_trilat n/a vs last_fix n/a"),
            "{reason}"
        );

        let defined_tie = evaluate_plausibility(
            Some(&previous),
            current,
            &[(0.0, 0.0), (0.0, 0.0), (0.0, 0.0)],
            now,
            exact_speed - 0.001,
        );
        assert_eq!(defined_tie.winner, PlausibilityWinner::CurrentTrilat);
        assert_eq!(defined_tie.quality.inlier_ratio(), Some(0.0));
        assert!(defined_tie
            .quality
            .suspect_reason()
            .is_some_and(|reason| reason.contains("score current_trilat 0.00 vs last_fix 0.00")));
    }

    #[test]
    fn signed_elapsed_seconds_preserves_sub_microsecond_sign_and_policy() {
        let now = chrono::Utc::now();
        let past = now - chrono::Duration::nanoseconds(500);
        let future = now + chrono::Duration::nanoseconds(500);
        let past_elapsed = signed_elapsed_seconds(now, past);
        let future_elapsed = signed_elapsed_seconds(now, future);
        assert!((past_elapsed - 5e-7).abs() < 1e-15, "got {past_elapsed}");
        assert!(
            (future_elapsed + 5e-7).abs() < 1e-15,
            "got {future_elapsed}"
        );

        let previous = LastFix {
            lat: 55.6050,
            lon: 13.0038,
            accuracy_m: 10.0,
            address: None,
            at: past,
            sources: 3,
        };
        let current = (55.6761, 12.5683);
        let positive_duration = evaluate_plausibility(
            Some(&previous),
            current,
            &[current, current, current],
            now,
            250.0,
        );
        assert_eq!(positive_duration.quality.confidence(), Confidence::Suspect);
        assert!(positive_duration.quality.implied_speed_kmh().unwrap() > 1_000_000_000.0);

        let future_previous = LastFix {
            at: future,
            ..previous
        };
        let future_clock = evaluate_plausibility(
            Some(&future_previous),
            current,
            &[current, current, current],
            now,
            1.0,
        );
        assert_eq!(future_clock.quality.implied_speed_kmh(), Some(0.0));
        assert_eq!(future_clock.quality.last_fix_age_secs(), Some(0));
        assert_eq!(future_clock.quality.confidence(), Confidence::High);
        assert_eq!(future_clock.winner, PlausibilityWinner::CurrentTrilat);
    }

    /// Synthetic cache row for the pure locate kernel. No DB, no scan — the
    /// kernel is a function of `(cached_aps, active, top_n)` alone (task-0116).
    fn kernel_ap(bssid: &str, lat: f64, lon: f64, is_mobile: bool) -> crate::db::ApInfo {
        crate::db::ApInfo {
            bssid: bssid.to_string(),
            ssid: Some("kernel-ap".to_string()),
            provider_reported_lat: lat,
            provider_reported_lon: lon,
            encryption: None,
            frequency: None,
            city: None,
            country: None,
            is_mobile,
            source: crate::db::Source::Manual,
        }
    }

    fn kernel_candidates(bssids: &[(&str, i32)]) -> Vec<Candidate> {
        bssids
            .iter()
            .map(|(bssid, signal_dbm)| Candidate {
                bssid: (*bssid).to_string(),
                signal_dbm: *signal_dbm,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                ssid: Some("kernel-ap".to_string()),
            })
            .collect()
    }

    fn active_index(candidates: &[Candidate]) -> std::collections::HashMap<&str, &Candidate> {
        candidates
            .iter()
            .map(|candidate| (candidate.bssid.as_str(), candidate))
            .collect()
    }

    /// AC#5: the pure kernel returns the expected Position from synthetic
    /// candidates + synthetic cache rows, with no DB access, no mutex, no
    /// spawn. Mirrors the cold-boot fixture cluster.
    #[test]
    fn locate_kernel_returns_expected_position_from_synthetic_rows() {
        let cluster = [
            ("b8:00:00:00:60:00", 55.6760, 12.5680),
            ("b8:00:00:00:60:01", 55.6761, 12.5682),
            ("b8:00:00:00:60:02", 55.6759, 12.5681),
        ];
        let cached_aps: Vec<crate::db::ApInfo> = cluster
            .iter()
            .map(|(bssid, lat, lon)| kernel_ap(bssid, *lat, *lon, false))
            .collect();
        let candidates = kernel_candidates(&[
            ("b8:00:00:00:60:00", -50),
            ("b8:00:00:00:60:01", -51),
            ("b8:00:00:00:60:02", -52),
        ]);
        let active = active_index(&candidates);

        let LocateKernel::Solved(solved) = solve_locate_from_cache(&cached_aps, &active, 8) else {
            panic!("three finite non-mobile cached APs must solve");
        };
        let pos = solved.result.expect("cluster must trilaterate");
        assert!(
            (pos.lat - 55.6760).abs() < 0.01 && (pos.lon - 12.5681).abs() < 0.01,
            "kernel position {pos:?} must land on the seeded cluster"
        );
        assert!(
            !pos.fallback,
            "a normal cluster is not the antipodal fallback"
        );
        assert!(
            pos.accuracy_m.is_finite() && pos.accuracy_m > 0.0,
            "accuracy must be a finite positive spread, got {}",
            pos.accuracy_m
        );
        assert_eq!(solved.source_count, 3, "all three eligible rows solve");
        assert_eq!(solved.cached_count, 3);
        assert_eq!(
            solved.visible_cached.len(),
            3,
            "consensus denominator counts every finite non-mobile cache hit"
        );
    }

    /// AC#5: the kernel's eligibility filters are pure and testable — non-finite
    /// and persisted-mobile rows are excluded from BOTH consensus and solver
    /// membership; a cached row absent from `active` still counts toward the
    /// consensus denominator but never becomes solver membership; and `top_n`
    /// caps membership without shrinking the denominator.
    #[test]
    fn locate_kernel_filters_ineligible_rows_and_respects_top_n() {
        let cached_aps = vec![
            kernel_ap("b8:00:00:00:61:00", 55.6760, 12.5680, false),
            kernel_ap("b8:00:00:00:61:01", 55.6761, 12.5682, false),
            // Present in the scan-consensus cache but NOT an active candidate:
            // contributes to visible_cached only.
            kernel_ap("b8:00:00:00:61:02", 55.6759, 12.5681, false),
            // Non-finite and persisted-mobile rows: excluded everywhere.
            kernel_ap("b8:00:00:00:61:03", f64::INFINITY, 12.5680, false),
            kernel_ap("b8:00:00:00:61:04", 55.6762, 12.5683, true),
        ];
        let candidates = kernel_candidates(&[
            ("b8:00:00:00:61:00", -50),
            ("b8:00:00:00:61:01", -51),
            // 61:03 / 61:04 are active too, proving the row-level (not
            // candidate-level) eligibility guard rejects them.
            ("b8:00:00:00:61:03", -40),
            ("b8:00:00:00:61:04", -39),
        ]);
        let active = active_index(&candidates);

        // top_n = 1 keeps only the first eligible membership row but must not
        // shrink the consensus denominator.
        let LocateKernel::Solved(solved) = solve_locate_from_cache(&cached_aps, &active, 1) else {
            panic!("two eligible fixed rows must solve");
        };
        assert_eq!(
            solved.source_count, 1,
            "top_n caps solver membership at one row"
        );
        assert_eq!(
            solved.visible_cached.len(),
            3,
            "consensus counts the three finite non-mobile rows; non-finite and \
             mobile are excluded, top_n does not shrink it"
        );

        // With no eligible cache hit at all, the kernel reports NoPositions and
        // leaves the cold-start policy to the caller.
        let barren = vec![kernel_ap("00:00:00:00:61:03", f64::NAN, 12.5680, false)];
        assert!(
            matches!(
                solve_locate_from_cache(&barren, &active, 8),
                LocateKernel::NoPositions
            ),
            "a non-finite-only cache must yield NoPositions"
        );
    }

    /// task-0122 AC#6: a tight cluster of Brazil-in-Copenhagen poisoners
    /// (locally-administered / randomized MACs, F6 first octet) is excluded
    /// from the solve entirely — BEFORE the outlier pipeline — so the kernel
    /// reports NoPositions instead of trilaterating a fake stay. It also never
    /// counts toward the consensus denominator.
    #[test]
    fn locate_kernel_excludes_locally_administered_poisoner_cluster() {
        let poisoners = [
            ("F6:B1:9C:0A:3A:60", 55.6760, 12.5680),
            ("F6:B1:9C:0A:3A:61", 55.6761, 12.5682),
            ("F6:B1:9C:0A:3A:62", 55.6759, 12.5681),
        ];
        let cached_aps: Vec<crate::db::ApInfo> = poisoners
            .iter()
            .map(|(bssid, lat, lon)| kernel_ap(bssid, *lat, *lon, false))
            .collect();
        let candidates = kernel_candidates(&[
            ("F6:B1:9C:0A:3A:60", -50),
            ("F6:B1:9C:0A:3A:61", -51),
            ("F6:B1:9C:0A:3A:62", -52),
        ]);
        let active = active_index(&candidates);

        assert!(
            matches!(
                solve_locate_from_cache(&cached_aps, &active, 8),
                LocateKernel::NoPositions
            ),
            "a cluster of randomized-MAC poisoners must be excluded before \
             drop_isolated, yielding NoPositions — never a trilaterated fix"
        );
    }

    /// task-0122 AC#7: universally-administered infrastructure BSSIDs (TP-Link
    /// OUI B8:50:01, U/L bit clear) are NOT excluded — the kernel trilaterates
    /// them exactly as before. Guards against the filter over-reaching into
    /// legitimate APs.
    #[test]
    fn locate_kernel_keeps_universally_administered_infra() {
        let infra = [
            ("B8:50:01:00:00:60", 55.6760, 12.5680),
            ("B8:50:01:00:00:61", 55.6761, 12.5682),
            ("B8:50:01:00:00:62", 55.6759, 12.5681),
        ];
        let cached_aps: Vec<crate::db::ApInfo> = infra
            .iter()
            .map(|(bssid, lat, lon)| kernel_ap(bssid, *lat, *lon, false))
            .collect();
        let candidates = kernel_candidates(&[
            ("B8:50:01:00:00:60", -50),
            ("B8:50:01:00:00:61", -51),
            ("B8:50:01:00:00:62", -52),
        ]);
        let active = active_index(&candidates);

        let LocateKernel::Solved(solved) = solve_locate_from_cache(&cached_aps, &active, 8) else {
            panic!("universally-administered infra APs must still solve");
        };
        let pos = solved.result.expect("infra cluster must trilaterate");
        assert!(
            (pos.lat - 55.6760).abs() < 0.01 && (pos.lon - 12.5681).abs() < 0.01,
            "infra position {pos:?} must land on the seeded cluster"
        );
        assert_eq!(solved.source_count, 3, "all three infra rows solve");
    }

    /// task-0122 AC#9: malformed BSSIDs are rejected before any provider or
    /// network call; well-formed ones (including locally-administered) are
    /// accepted and normalized to canonical uppercase.
    #[test]
    fn validate_resolve_bssids_rejects_malformed_accepts_wellformed() {
        for bad in [
            "00:00:00:00:00:00", // all zeros
            "ff:ff:ff:ff:ff:ff", // broadcast
            "01:00:5e:00:00:01", // multicast
            "aa:bb:cc:dd:ee",    // too short
            "zz:bb:cc:dd:ee:ff", // non-hex
            "aa&x=1",            // injection-shaped
            "../",               // path traversal
        ] {
            assert!(
                validate_resolve_bssids(&[bad.to_string()]).is_err(),
                "malformed BSSID {bad:?} must be rejected before any network call"
            );
        }

        // Well-formed inputs pass and are canonicalized. A locally-administered
        // MAC is well-formed and accepted for resolve (the randomized-MAC
        // exclusion is a trilateration concern, not a resolve one).
        assert_eq!(
            validate_resolve_bssids(&["aa:bb:cc:dd:ee:ff".to_string()]).unwrap(),
            vec!["AA:BB:CC:DD:EE:FF".to_string()]
        );
        assert!(
            validate_resolve_bssids(&["f6:b1:9c:0a:3a:60".to_string()]).is_ok(),
            "a well-formed locally-administered MAC is a valid resolve target"
        );
    }

    /// task-0162: `handle_resolve` caps a request at `MAX_RESOLVE_BSSIDS`
    /// (256). Firing 257 well-formed BSSIDs must be rejected with the
    /// "accepts at most" error BEFORE any provider/network call — the length
    /// guard precedes both `validate_resolve_bssids` and `resolve_readonly`, so
    /// this runs fully offline (empty WiGLE creds, no quota touched). Guards the
    /// unauthenticated-request-size ceiling the wire protocol advertises.
    #[tokio::test]
    async fn handle_resolve_rejects_more_than_max_bssids() {
        let state = test_state();
        // 257 distinct, well-formed 6-octet MACs (index encoded in last 2 octets).
        let bssids: Vec<String> = (0..=MAX_RESOLVE_BSSIDS)
            .map(|i| format!("b8:00:00:00:{:02x}:{:02x}", (i >> 8) & 0xff, i & 0xff))
            .collect();
        assert_eq!(bssids.len(), MAX_RESOLVE_BSSIDS + 1);

        let response = handle_resolve(&bssids, &state, state.privacy.ticket()).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], false, "over-cap resolve must fail: {response}");
        let error = json["error"].as_str().unwrap_or_default();
        assert!(
            error.contains("at most") && error.contains(&MAX_RESOLVE_BSSIDS.to_string()),
            "error must name the {MAX_RESOLVE_BSSIDS} cap, got: {error}"
        );
    }

    #[tokio::test]
    async fn locate_cold_boot_reports_consensus_without_speed_fields() {
        let state = test_state();
        let mut sample = crate::debounce::ScanSample::new();
        for (index, (lat, lon)) in [(55.6760, 12.5680), (55.6761, 12.5682), (55.6759, 12.5681)]
            .into_iter()
            .enumerate()
        {
            add_cached_scan_ap(
                &state,
                &mut sample,
                &format!("b8:00:00:00:60:{index:02x}"),
                lat,
                lon,
                -50 - index as i32,
                Some("fixed-home"),
            );
        }
        push_stable_scan(&state, sample).await;

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], true, "{response}");
        assert_eq!(json["confidence"], "high");
        assert_eq!(json["inlier_ratio"], 1.0);
        for field in ["implied_speed_kmh", "last_fix_age_secs", "suspect_reason"] {
            assert!(
                json.get(field).is_none(),
                "cold boot must omit {field}: {response}"
            );
        }
        // A normal fix is not the antipodal fallback: the skip-when-false field
        // must be absent (task-0127), decoding to false on the client.
        assert!(
            json.get("fallback").is_none(),
            "normal locate must omit fallback: {response}"
        );
    }

    /// task-0162 (relates task-0046): an interactive `locate` that produces a
    /// fix must WRITE THROUGH to BOTH the authoritative `last_fix` row AND the
    /// `fixes` history timeseries. This is the cold-boot / first-fix path (no
    /// previous row, high-confidence CurrentTrilat winner) that the existing
    /// `suspect_current_winner_preserves_address_last_fix_and_history_semantics`
    /// only exercises for the current-over-previous branch. A regression where
    /// `handle_locate` stopped persisting on the first fix (the task-0046 write
    /// hazard) would leave both tables empty here while the JSON still says ok.
    /// Uses cached APs only — no provider/network call, no live daemon.
    #[tokio::test]
    async fn locate_cold_boot_writes_through_to_last_fix_and_history() {
        let state = test_state();
        // No previous last_fix seeded: this is the cold-boot write-through.
        assert!(
            state.db.get_last_fix().await.unwrap().is_none(),
            "precondition: cold boot has no persisted last_fix"
        );
        assert!(
            history_rows(&state).await.is_empty(),
            "precondition: cold boot has empty history"
        );

        let mut sample = crate::debounce::ScanSample::new();
        for (index, (lat, lon)) in [(55.6760, 12.5680), (55.6761, 12.5682), (55.6759, 12.5681)]
            .into_iter()
            .enumerate()
        {
            add_cached_scan_ap(
                &state,
                &mut sample,
                &format!("b8:00:00:00:62:{index:02x}"),
                lat,
                lon,
                -50 - index as i32,
                Some("fixed-home"),
            );
        }
        push_stable_scan(&state, sample).await;

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], true, "{response}");

        // Write-through assertion: BOTH rows must exist after the locate.
        let stored = state
            .db
            .get_last_fix()
            .await
            .unwrap()
            .expect("locate must persist a last_fix row");
        assert!((stored.lat - json["lat"].as_f64().unwrap()).abs() < 1e-9);
        assert!((stored.lon - json["lon"].as_f64().unwrap()).abs() < 1e-9);

        let rows = history_rows(&state).await;
        assert_eq!(
            rows.len(),
            1,
            "locate must append exactly one fixes history row"
        );
    }

    #[tokio::test]
    async fn locate_antipodal_cached_aps_report_fallback_true() {
        // task-0127 AC#5 at the dispatch level: two equal-weight, exactly
        // antipodal cached APs trip trilaterate's cancellation guard. The wire
        // response must carry fallback:true and the 1000 m sentinel accuracy,
        // out-of-band from any legitimately-wide fix.
        let state = test_state();
        let mut sample = crate::debounce::ScanSample::new();
        add_cached_scan_ap(
            &state,
            &mut sample,
            "b8:00:00:00:7f:00",
            0.0,
            0.0,
            -50,
            Some("a"),
        );
        add_cached_scan_ap(
            &state,
            &mut sample,
            "b8:00:00:00:7f:01",
            0.0,
            180.0,
            -50,
            Some("b"),
        );
        push_stable_scan(&state, sample).await;

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], true, "{response}");
        assert_eq!(
            json["fallback"], true,
            "antipodal cached APs must report fallback=true: {response}"
        );
        assert_eq!(
            json["accuracy_m"].as_f64().unwrap(),
            1000.0,
            "fallback sentinel accuracy must be 1000: {response}"
        );
    }

    #[tokio::test]
    async fn antipodal_fallback_fix_does_not_overwrite_last_fix() {
        // task-0263: an antipodal-fallback fix (accuracy_m=1000 sentinel) must
        // NOT become the authoritative last_fix. Otherwise a later stale read
        // would serve ~1000 m with fallback=false, re-introducing the in-band
        // ambiguity task-0127 removed. The prior real last_fix must survive
        // unchanged, while the fixes timeseries still records the observation.
        let state = test_state();

        // Prior authoritative fix at (0,180) — the strongest-AP the guard echoes
        // for the antipodal pair below — so the CurrentTrilat fallback wins the
        // plausibility check (distance 0) and reaches the persist site under test.
        let previous = previous_row(0.0, 180.0, chrono::Duration::minutes(5));
        state.db.set_last_fix(previous.clone()).await.unwrap();

        let mut sample = crate::debounce::ScanSample::new();
        add_cached_scan_ap(
            &state,
            &mut sample,
            "b8:00:00:00:7f:00",
            0.0,
            0.0,
            -50,
            Some("a"),
        );
        add_cached_scan_ap(
            &state,
            &mut sample,
            "b8:00:00:00:7f:01",
            0.0,
            180.0,
            -50,
            Some("b"),
        );
        push_stable_scan(&state, sample).await;

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], true, "{response}");
        assert_eq!(
            json["fallback"], true,
            "the winning fix must be the antipodal fallback: {response}"
        );
        assert_eq!(
            json["accuracy_m"].as_f64().unwrap(),
            1000.0,
            "fallback sentinel accuracy must be 1000: {response}"
        );

        // The authoritative last_fix is the PRIOR real fix, untouched: same
        // timestamp (no invented motion), same non-sentinel accuracy, same
        // address/sources. A regression here would cache the 1000 m sentinel.
        let stored = state.db.get_last_fix().await.unwrap().unwrap();
        assert_eq!(
            stored.at, previous.at,
            "fallback fix must not refresh the last_fix timestamp"
        );
        assert_eq!(
            stored.accuracy_m, previous.accuracy_m,
            "fallback 1000 m sentinel must not overwrite the prior last_fix accuracy"
        );
        assert!((stored.lat - previous.lat).abs() < 1e-12);
        assert!((stored.lon - previous.lon).abs() < 1e-12);
        assert_eq!(stored.address, previous.address);
        assert_eq!(stored.sources, previous.sources);

        // History is a separate raw timeseries and DOES record the fallback
        // observation. Its presence also proves we took the CurrentTrilat
        // fallback path (a last_fix winner writes no history).
        let rows = history_rows(&state).await;
        assert_eq!(
            rows.len(),
            1,
            "fallback observation must be recorded in history"
        );
        assert_eq!(
            rows[0].accuracy_m, 1000.0,
            "history must record the fallback observation's sentinel accuracy"
        );
    }

    #[tokio::test]
    async fn speed_alarm_uses_full_visible_consensus_and_keeps_previous_row_literal() {
        const PREVIOUS: (f64, f64) = (55.7047, 12.5900);
        const BRAZIL: (f64, f64) = (-12.8940, -38.2920);

        let mut state = test_state();
        Arc::get_mut(&mut state)
            .expect("state must be uniquely owned before test configuration")
            .args
            .top_n = 1;
        let previous = previous_row(PREVIOUS.0, PREVIOUS.1, chrono::Duration::minutes(5));
        state.db.set_last_fix(previous.clone()).await.unwrap();

        let mut sample = crate::debounce::ScanSample::new();
        add_cached_scan_ap(
            &state,
            &mut sample,
            "b8:00:00:00:61:ff",
            BRAZIL.0,
            BRAZIL.1,
            -20,
            Some("strong-outlier"),
        );
        for index in 0..4 {
            add_cached_scan_ap(
                &state,
                &mut sample,
                &format!("b8:00:00:00:61:{index:02x}"),
                PREVIOUS.0 + index as f64 / 100_000.0,
                PREVIOUS.1 + index as f64 / 100_000.0,
                -60 - index,
                Some("fixed-near-previous"),
            );
        }
        push_stable_scan(&state, sample).await;

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], true, "{response}");
        assert_eq!(json["confidence"], "suspect");
        assert_eq!(
            json["cached"], 1,
            "top_n=1 must still govern solver metadata"
        );
        assert_eq!(json["sources"], 1);
        assert_eq!(json["visible"], 5);
        assert_eq!(json["stable"], 5);
        assert_eq!(
            json["inlier_ratio"], 0.8,
            "consensus denominator must include all five visible cached APs"
        );
        assert!((json["lat"].as_f64().unwrap() - PREVIOUS.0).abs() < 1e-12);
        assert!((json["lon"].as_f64().unwrap() - PREVIOUS.1).abs() < 1e-12);
        assert_eq!(json["address"], "previous address");
        assert!(json["implied_speed_kmh"].as_f64().unwrap() > 250.0);
        let expected_accuracy = previous.accuracy_m
            + crate::geo::haversine_m(PREVIOUS.0, PREVIOUS.1, BRAZIL.0, BRAZIL.1);
        assert!(
            (json["accuracy_m"].as_f64().unwrap() - expected_accuracy).abs() < 1e-6,
            "previous winner must add current-candidate distance to saved accuracy"
        );
        let reason = json["suspect_reason"].as_str().unwrap();
        assert!(reason.contains("tie-break favored last_fix"), "{reason}");
        assert!(
            reason.contains("score last_fix 0.80 vs current_trilat 0.20"),
            "{reason}"
        );
        assert!(json.get("stale").is_none() && json.get("age_s").is_none());

        let stored = state.db.get_last_fix().await.unwrap().unwrap();
        assert_eq!(
            stored.at, previous.at,
            "policy reuse must not refresh the observation timestamp"
        );
        assert_eq!(
            stored.accuracy_m, previous.accuracy_m,
            "drift bound must not compound on disk"
        );
        assert!(
            state
                .db
                .get_fixes_in_range("0000", "9999")
                .await
                .unwrap()
                .is_empty(),
            "last_fix winner must not append synthetic history"
        );
    }

    #[tokio::test]
    async fn suspect_current_winner_preserves_address_last_fix_and_history_semantics() {
        const CURRENT: (f64, f64) = (55.6761, 12.5683);
        const PREVIOUS: (f64, f64) = (-12.8940, -38.2920);
        let mut state = test_state();
        Arc::get_mut(&mut state)
            .expect("state must be uniquely owned before test configuration")
            .args
            .address_approx = true;
        state
            .db
            .set_last_fix(previous_row(
                PREVIOUS.0,
                PREVIOUS.1,
                chrono::Duration::minutes(5),
            ))
            .await
            .unwrap();
        state.address_cache.lock().unwrap().insert(
            CURRENT.0,
            CURRENT.1,
            "current address".to_string(),
        );

        let mut sample = crate::debounce::ScanSample::new();
        for index in 0..4 {
            add_cached_scan_ap(
                &state,
                &mut sample,
                &format!("b8:00:00:00:65:{index:02x}"),
                CURRENT.0,
                CURRENT.1,
                -40 - index,
                Some("fixed-current"),
            );
        }
        add_cached_scan_ap(
            &state,
            &mut sample,
            "b8:00:00:00:65:ff",
            PREVIOUS.0,
            PREVIOUS.1,
            -80,
            Some("fixed-previous"),
        );
        push_stable_scan(&state, sample).await;

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["confidence"], "suspect", "{response}");
        assert_eq!(json["inlier_ratio"], 0.8);
        assert!((json["lat"].as_f64().unwrap() - CURRENT.0).abs() < 1e-12);
        assert!((json["lon"].as_f64().unwrap() - CURRENT.1).abs() < 1e-12);
        assert_eq!(json["address"], "current address");
        assert!(json["suspect_reason"]
            .as_str()
            .unwrap()
            .contains("score current_trilat 0.80 vs last_fix 0.20"));

        let stored = state.db.get_last_fix().await.unwrap().unwrap();
        assert!((stored.lat - CURRENT.0).abs() < 1e-12);
        assert!((stored.lon - CURRENT.1).abs() < 1e-12);
        assert_eq!(stored.address.as_deref(), Some("current address"));
        assert_eq!(
            state
                .db
                .get_fixes_in_range("0000", "9999")
                .await
                .unwrap()
                .len(),
            1
        );
        let rows = history_rows(&state).await;
        let quality = rows[0]
            .quality
            .as_ref()
            .expect("current winner history must include quality");
        assert_eq!(quality.confidence(), Confidence::Suspect);
        // Suspect speeds are large (>250 km/h), so the ULP-scale JSON re-parse
        // error needs the same relative tolerance as the high-confidence case.
        assert_speed_matches(
            quality.implied_speed_kmh(),
            json["implied_speed_kmh"].as_f64(),
        );
        assert_eq!(
            quality.last_fix_age_secs(),
            json["last_fix_age_secs"].as_i64()
        );
        assert_eq!(quality.inlier_ratio(), json["inlier_ratio"].as_f64());
        assert_eq!(
            quality.suspect_reason(),
            json["suspect_reason"].as_str(),
            "response and persistence must use the same decision bundle"
        );
        let memory = state.last_fix.lock().await;
        let memory = memory
            .as_ref()
            .expect("current winner must publish in memory");
        assert!((memory.lat - CURRENT.0).abs() < 1e-12);
        assert_eq!(memory.address.as_deref(), Some("current address"));
    }

    #[tokio::test]
    async fn legitimate_drive_keeps_current_high_confidence_fix() {
        const MALMOE: (f64, f64) = (55.6050, 13.0038);
        const COPENHAGEN: (f64, f64) = (55.6761, 12.5683);
        let state = test_state();
        state
            .db
            .set_last_fix(previous_row(
                MALMOE.0,
                MALMOE.1,
                chrono::Duration::minutes(30),
            ))
            .await
            .unwrap();
        let mut sample = crate::debounce::ScanSample::new();
        for index in 0..3 {
            add_cached_scan_ap(
                &state,
                &mut sample,
                &format!("b8:00:00:00:62:{index:02x}"),
                COPENHAGEN.0 + index as f64 / 100_000.0,
                COPENHAGEN.1 + index as f64 / 100_000.0,
                -50 - index,
                Some("fixed-current"),
            );
        }
        push_stable_scan(&state, sample).await;

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["confidence"], "high", "{response}");
        assert_eq!(json["inlier_ratio"], 1.0);
        assert!((50.0..65.0).contains(&json["implied_speed_kmh"].as_f64().unwrap()));
        assert!((1_790..=1_810).contains(&json["last_fix_age_secs"].as_i64().unwrap()));
        assert!(json.get("suspect_reason").is_none());
        assert!((json["lat"].as_f64().unwrap() - COPENHAGEN.0).abs() < 0.001);
        assert_eq!(
            state
                .db
                .get_fixes_in_range("0000", "9999")
                .await
                .unwrap()
                .len(),
            1
        );
        let rows = history_rows(&state).await;
        let quality = rows[0].quality.as_ref().unwrap();
        assert_eq!(quality.confidence(), Confidence::High);
        // Age is i64 (exact) and inlier_ratio is pinned to 1.0 (exact under any
        // parser); only the speed needs tolerance — see assert_speed_matches.
        assert_speed_matches(
            quality.implied_speed_kmh(),
            json["implied_speed_kmh"].as_f64(),
        );
        assert_eq!(
            quality.last_fix_age_secs(),
            json["last_fix_age_secs"].as_i64()
        );
        assert_eq!(quality.inlier_ratio(), json["inlier_ratio"].as_f64());
    }

    #[tokio::test]
    async fn locate_with_two_visible_cached_aps_forces_low_confidence() {
        let state = test_state();
        let mut sample = crate::debounce::ScanSample::new();
        for index in 0..2 {
            add_cached_scan_ap(
                &state,
                &mut sample,
                &format!("b8:00:00:00:63:{index:02x}"),
                55.6761 + index as f64 / 100_000.0,
                12.5683 + index as f64 / 100_000.0,
                -50 - index,
                Some("fixed-current"),
            );
        }
        push_stable_scan(&state, sample).await;

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], true, "{response}");
        assert_eq!(json["confidence"], "low");
        assert!(json.get("inlier_ratio").is_none());
        let rows = history_rows(&state).await;
        let quality = rows[0].quality.as_ref().unwrap();
        assert_eq!(quality.confidence(), Confidence::Low);
        assert_eq!(quality.implied_speed_kmh(), None);
        assert_eq!(quality.last_fix_age_secs(), None);
        assert_eq!(quality.inlier_ratio(), None);
    }

    /// A plausible speed is necessary but not sufficient: mobile hardware can
    /// evade the coarse SSID taxonomy when the train hides its SSID or uses an
    /// unrecognized name. A coherent moved cluster then earns high consensus,
    /// and a slow enough apparent displacement is intentionally not flagged.
    /// A consensus-only mode that can challenge plausible-speed clusters is a
    /// follow-up, not behavior implemented by this speed-gated policy.
    #[tokio::test]
    async fn undocumented_train_ssids_can_still_produce_wrong_high_confidence_fix() {
        const STRANDBOULEVARDEN: (f64, f64) = (55.7047, 12.5900);
        const MAGLEVAENGET: (f64, f64) = (55.7390, 12.5500);
        let state = test_state();
        state
            .db
            .set_last_fix(previous_row(
                STRANDBOULEVARDEN.0,
                STRANDBOULEVARDEN.1,
                chrono::Duration::minutes(30),
            ))
            .await
            .unwrap();
        let endpoint_delta_lat = (175.0_f64 / 6_371_000.0).to_degrees();
        let train_coords = [
            (MAGLEVAENGET.0 - endpoint_delta_lat, MAGLEVAENGET.1),
            MAGLEVAENGET,
            (MAGLEVAENGET.0 + endpoint_delta_lat, MAGLEVAENGET.1),
        ];
        let endpoint_span = crate::geo::haversine_m(
            train_coords[0].0,
            train_coords[0].1,
            train_coords[2].0,
            train_coords[2].1,
        );
        assert!(
            (349.0..=351.0).contains(&endpoint_span),
            "train AP fixture must span about 350m, got {endpoint_span}m"
        );
        let mut sample = crate::debounce::ScanSample::new();
        for (index, ssid) in [None, Some("RailNet"), Some("")].into_iter().enumerate() {
            add_cached_scan_ap(
                &state,
                &mut sample,
                &format!("b8:00:00:00:64:{index:02x}"),
                train_coords[index].0,
                train_coords[index].1,
                -50,
                ssid,
            );
        }
        push_stable_scan(&state, sample).await;

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(
            json["confidence"], "high",
            "documented limitation changed: {response}"
        );
        assert_eq!(json["inlier_ratio"], 1.0);
        let speed = json["implied_speed_kmh"].as_f64().unwrap();
        assert!(
            (speed - 9.1).abs() < 1.0,
            "fixture must remain a plausible train displacement: {speed}"
        );
        let candidate = (json["lat"].as_f64().unwrap(), json["lon"].as_f64().unwrap());
        assert!((candidate.0 - MAGLEVAENGET.0).abs() < 0.001);
        assert!((candidate.1 - MAGLEVAENGET.1).abs() < 0.001);
        for ap in train_coords {
            let distance = crate::geo::haversine_m(candidate.0, candidate.1, ap.0, ap.1);
            assert!(
                distance <= 200.0,
                "every train AP must be an inclusive-200m consensus inlier; got {distance}m"
            );
        }
        assert!(json.get("suspect_reason").is_none());
    }

    #[tokio::test]
    async fn stale_fallback_reports_low_uncertainty_and_clamps_future_age() {
        let state = test_state();
        *state.last_fix.lock().await = Some(LastFix {
            lat: 55.6761,
            lon: 12.5683,
            accuracy_m: 20.0,
            address: None,
            at: chrono::Utc::now() + chrono::Duration::minutes(5),
            sources: 3,
        });

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], true, "{response}");
        assert_eq!(json["stale"], true);
        assert_eq!(json["age_s"], 0);
        assert_eq!(json["last_fix_age_secs"], 0);
        assert_eq!(json["confidence"], "low");
        for field in ["implied_speed_kmh", "inlier_ratio", "suspect_reason"] {
            assert!(
                json.get(field).is_none(),
                "stale fallback must omit {field}: {response}"
            );
        }
    }

    #[test]
    fn fallback_resolution_skips_cached_rows_before_applying_top_n_bound() {
        let cached: std::collections::HashSet<String> = (0..11)
            .map(|index| format!("cached-mobile-{index}"))
            .collect();
        let candidates: Vec<Candidate> = cached
            .iter()
            .cloned()
            .map(|bssid| Candidate {
                bssid,
                signal_dbm: -10,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                ssid: None,
            })
            .chain((0..12).map(|index| Candidate {
                bssid: format!("unknown-fixed-{index}"),
                signal_dbm: -40 - index,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                ssid: Some(format!("fixed-{index}")),
            }))
            .collect();

        let observations = bounded_uncached_observations(&candidates, &cached, 10);
        assert_eq!(observations.len(), 10, "provider work must stay bounded");
        assert!(observations
            .iter()
            .all(|observation| !cached.contains(&observation.bssid)));
        let observed: std::collections::HashSet<&str> = observations
            .iter()
            .map(|observation| observation.bssid.as_str())
            .collect();
        for index in 0..10 {
            let expected = format!("unknown-fixed-{index}");
            assert!(observed.contains(expected.as_str()));
        }
    }

    async fn seed_wipe_state(state: &Arc<DaemonState>) {
        cache_ap(state, "aa:bb:cc:dd:ee:01", 55.0, 12.0);
        {
            let db = lock_db(state);
            db.insert_pending("pending", Some("private-ssid"), None, Some(-40))
                .unwrap();
            db.insert_not_found("not-found").unwrap();
            db.insert_fix("2026-01-01T00:00:00+00:00", 55.0, 12.0, 5.0, 1)
                .unwrap();
            db.set_last_fix(&LastFix {
                lat: 55.0,
                lon: 12.0,
                accuracy_m: 5.0,
                address: Some("private address".to_string()),
                at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                sources: 1,
            })
            .unwrap();
        }
        let mut sample = std::collections::HashMap::new();
        sample.insert(
            "aa:bb:cc:dd:ee:01".to_string(),
            crate::debounce::ScanEntry {
                rssi_source: crate::rssi::RssiSource::IwDbm,
                signal_dbm: -40,
                ssid: Some("private-ssid".to_string()),
                channel: Some(whereami_protocol::Channel::TwoFourGhz(6)),
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
            crate::wigle::WigleClient::new("", ""),
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
                provider_reported_lat: 55.0,
                provider_reported_lon: 12.0,
                encryption: None,
                frequency: None,
                city: None,
                country: None,
                is_mobile: false,
                source: crate::db::Source::Manual,
            })
            .unwrap();
            db.insert_pending("pending", Some("private-ssid"), None, Some(-40))
                .unwrap();
            db.insert_not_found("not-found").unwrap();
            db.insert_fix("2026-01-01T00:00:00+00:00", 55.0, 12.0, 5.0, 1)
                .unwrap();
            db.set_last_fix(&LastFix {
                lat: 55.0,
                lon: 12.0,
                accuracy_m: 5.0,
                address: Some("private address".to_string()),
                at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
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
            .map(|index| format!("b8:00:00:00:10:{index:02x}"))
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
                        rssi_source: crate::rssi::RssiSource::IwDbm,
                        signal_dbm: -30 - index as i32,
                        ssid: None,
                        channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
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
    async fn locate_excludes_mobile_cache_rows_and_reports_only_fixed_sources() {
        let state = test_state();
        let rows = [
            ("b8:00:00:00:30:01", "home-a", 55.6760, 12.5680),
            ("b8:00:00:00:30:02", "home-b", 55.6761, 12.5682),
            ("b8:00:00:00:30:03", "home-c", 55.6759, 12.5681),
            ("b8:00:00:00:30:04", "MOVIA_BUS_42", 55.7390, 12.5500),
            ("b8:00:00:00:30:05", "DSB_Wi-Fi", 55.7391, 12.5501),
        ];
        let mut sample = crate::debounce::ScanSample::new();
        for (index, (bssid, ssid, lat, lon)) in rows.iter().enumerate() {
            lock_db(&state)
                .upsert_ap(&crate::db::ApInfo {
                    bssid: (*bssid).to_owned(),
                    ssid: Some((*ssid).to_owned()),
                    provider_reported_lat: *lat,
                    provider_reported_lon: *lon,
                    encryption: None,
                    frequency: None,
                    city: None,
                    country: None,
                    is_mobile: false,
                    source: crate::db::Source::Apple,
                })
                .unwrap();
            sample.insert(
                (*bssid).to_owned(),
                crate::debounce::ScanEntry {
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    signal_dbm: -50 - index as i32,
                    ssid: Some((*ssid).to_owned()),
                    channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
                },
            );
        }
        {
            let mut debouncer = state.debouncer.lock().await;
            for _ in 0..state.args.debounce_threshold {
                debouncer.push_scan(sample.clone());
            }
        }

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], true, "locate failed: {response}");
        assert_eq!(json["cached"], 3);
        assert_eq!(json["sources"], 3);
        let lat = json["lat"].as_f64().unwrap();
        let lon = json["lon"].as_f64().unwrap();
        assert!(
            (lat - 55.6760).abs() < 0.001,
            "mobile APs pulled lat: {lat}"
        );
        assert!(
            (lon - 12.5681).abs() < 0.001,
            "mobile APs pulled lon: {lon}"
        );
        assert_eq!(state.db.get_last_fix().await.unwrap().unwrap().sources, 3);
    }

    #[tokio::test]
    async fn stronger_mobile_scans_cannot_starve_weaker_fixed_top_n_candidates() {
        let state = test_state();
        assert_eq!(state.args.top_n, 10);
        let mut sample = crate::debounce::ScanSample::new();

        // Eleven stronger mobile APs exceed top_n by themselves. Filtering
        // after truncation would therefore leave no fixed candidate at all.
        for index in 0..11 {
            let bssid = format!("b8:00:00:00:40:{index:02x}");
            let ssid = format!("MOVIA_BUS_{index}");
            lock_db(&state)
                .upsert_ap(&crate::db::ApInfo {
                    bssid: bssid.clone(),
                    ssid: Some(ssid.clone()),
                    provider_reported_lat: 55.7390 + index as f64 / 100_000.0,
                    provider_reported_lon: 12.5500,
                    encryption: None,
                    frequency: None,
                    city: None,
                    country: None,
                    is_mobile: false,
                    source: crate::db::Source::Apple,
                })
                .unwrap();
            sample.insert(
                bssid,
                crate::debounce::ScanEntry {
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    signal_dbm: -20 - index,
                    ssid: Some(ssid),
                    channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
                },
            );
        }
        for (index, (lat, lon)) in [(55.6760, 12.5680), (55.6761, 12.5682), (55.6759, 12.5681)]
            .into_iter()
            .enumerate()
        {
            let bssid = format!("b8:00:00:00:41:{index:02x}");
            let ssid = format!("fixed-home-{index}");
            lock_db(&state)
                .upsert_ap(&crate::db::ApInfo {
                    bssid: bssid.clone(),
                    ssid: Some(ssid.clone()),
                    provider_reported_lat: lat,
                    provider_reported_lon: lon,
                    encryption: None,
                    frequency: None,
                    city: None,
                    country: None,
                    is_mobile: false,
                    source: crate::db::Source::Apple,
                })
                .unwrap();
            sample.insert(
                bssid,
                crate::debounce::ScanEntry {
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    signal_dbm: -80 - index as i32,
                    ssid: Some(ssid),
                    channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
                },
            );
        }
        {
            let mut debouncer = state.debouncer.lock().await;
            for _ in 0..state.args.debounce_threshold {
                debouncer.push_scan(sample.clone());
            }
        }

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], true, "locate failed: {response}");
        assert_eq!(json["cached"], 3);
        assert_eq!(json["sources"], 3);
        assert!((json["lat"].as_f64().unwrap() - 55.6760).abs() < 0.001);
        assert!((json["lon"].as_f64().unwrap() - 12.5681).abs() < 0.001);
    }

    #[tokio::test]
    async fn persisted_mobile_and_non_finite_rows_cannot_starve_stable_top_n() {
        let state = test_state();
        assert_eq!(state.args.top_n, 10);
        let sample = cached_ineligible_rows_ahead_of_fixed(&state, 0x50);
        assert!(sample.len() > state.args.top_n * 2);
        {
            let mut debouncer = state.debouncer.lock().await;
            for _ in 0..state.args.debounce_threshold {
                debouncer.push_scan(sample.clone());
            }
        }

        let response = dispatch_command(&Request::Locate, &state).await;
        assert_three_fixed_sources(&response);
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["visible"], 25);
        assert_eq!(json["stable"], 25);
    }

    #[tokio::test]
    async fn locate_skips_non_finite_cached_ap_and_warns_with_bssid() {
        use tracing::instrument::WithSubscriber as _;

        // Root-cause de-flake (task-0248): force the global interest cache to
        // keep this WARN callsite at `sometimes` so the scoped capture below can
        // never lose the event to a cross-thread `never`-clobber under parallel
        // `cargo test` load. See install_permissive_global_test_subscriber.
        crate::test_support::install_permissive_global_test_subscriber();

        const POISONED_BSSID: &str = "b8:00:00:00:52:ff";
        let state = test_state();
        assert_eq!(state.args.top_n, 10);

        cache_ap(&state, POISONED_BSSID, f64::INFINITY, 12.56);
        let mut sample = crate::debounce::ScanSample::new();
        sample.insert(
            POISONED_BSSID.to_owned(),
            crate::debounce::ScanEntry {
                rssi_source: crate::rssi::RssiSource::IwDbm,
                signal_dbm: -20,
                ssid: Some("poisoned-fixed".to_owned()),
                channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
            },
        );
        for (index, (lat, lon)) in [(55.6760, 12.5680), (55.6761, 12.5682), (55.6759, 12.5681)]
            .into_iter()
            .enumerate()
        {
            let bssid = format!("b8:00:00:00:52:{index:02x}");
            cache_ap(&state, &bssid, lat, lon);
            sample.insert(
                bssid,
                crate::debounce::ScanEntry {
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    signal_dbm: -60 - index as i32,
                    ssid: Some(format!("fixed-home-{index}")),
                    channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
                },
            );
        }
        {
            let mut debouncer = state.debouncer.lock().await;
            for _ in 0..state.args.debounce_threshold {
                debouncer.push_scan(sample.clone());
            }
        }

        // Attach the subscriber to the future itself: a thread-local guard
        // around future construction would end before the async handler is
        // polled and could silently miss the production warning.
        // task-0255: raw shared CaptureSink; async site filters on WARN +
        // target, so it keeps the unified (level, target, message) tuple.
        let captured = crate::test_support::CaptureSink::default();
        let handle = captured.clone();
        let response = dispatch_command(&Request::Locate, &state)
            .with_subscriber(captured)
            .await;

        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], true, "locate failed: {response}");
        assert_eq!(json["cached"], 3, "the poisoned row must not be admitted");
        assert_eq!(json["sources"], 3, "the poisoned row must not be a source");
        assert_eq!(json["visible"], 4);
        assert_eq!(json["stable"], 4);
        assert!((json["lat"].as_f64().unwrap() - 55.6760).abs() < 0.001);
        assert!((json["lon"].as_f64().unwrap() - 12.5681).abs() < 0.001);

        let events = handle.events();
        let server_target = module_path!()
            .strip_suffix("::dispatch_tests")
            .expect("test module path must end in dispatch_tests");
        let warnings: Vec<_> = events
            .iter()
            .filter(|(level, target, _)| *level == tracing::Level::WARN && target == server_target)
            .map(|(_, _, message)| message.as_str())
            .collect();
        assert_eq!(
            warnings,
            [format!(
                "skipping AP {POISONED_BSSID}: non-finite cached position (inf, 12.56)"
            )],
            "captured events: {events:?}"
        );
    }

    #[tokio::test]
    async fn persisted_mobile_and_non_finite_rows_cannot_starve_fallback_top_n() {
        let state = test_state();
        assert!(state.args.debounce_threshold > 1);
        let sample = cached_ineligible_rows_ahead_of_fixed(&state, 0x51);
        state.debouncer.lock().await.push_scan(sample);

        let response = dispatch_command(&Request::Locate, &state).await;
        assert_three_fixed_sources(&response);
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["visible"], 25);
        assert_eq!(
            json["stable"], 0,
            "one scan must exercise cold-start fallback"
        );
    }

    #[tokio::test]
    async fn all_mobile_cold_start_fallback_cannot_fabricate_a_position() {
        let state = test_state();
        assert!(state.args.debounce_threshold > 1);
        let mut sample = crate::debounce::ScanSample::new();
        for index in 0..3 {
            let bssid = format!("b8:00:00:00:42:{index:02x}");
            let ssid = format!("MOVIA_BUS_{index}");
            lock_db(&state)
                .upsert_ap(&crate::db::ApInfo {
                    bssid: bssid.clone(),
                    ssid: Some(ssid.clone()),
                    provider_reported_lat: 55.7,
                    provider_reported_lon: 12.5,
                    encryption: None,
                    frequency: None,
                    city: None,
                    country: None,
                    is_mobile: false,
                    source: crate::db::Source::Apple,
                })
                .unwrap();
            sample.insert(
                bssid,
                crate::debounce::ScanEntry {
                    rssi_source: crate::rssi::RssiSource::IwDbm,
                    signal_dbm: -20 - index,
                    ssid: Some(ssid),
                    channel: Some(whereami_protocol::Channel::TwoFourGhz(1)),
                },
            );
        }
        state.debouncer.lock().await.push_scan(sample);

        let response = dispatch_command(&Request::Locate, &state).await;
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], false, "all-mobile scan must fail: {response}");
        assert_eq!(json["error"], "no eligible fixed APs detected");
        assert!(state.db.get_last_fix().await.unwrap().is_none());
        assert!(state
            .db
            .get_fixes_in_range("0000", "9999")
            .await
            .unwrap()
            .is_empty());
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

        let bssid = "b8:00:00:00:20:01";
        let mut sample = crate::debounce::ScanSample::new();
        sample.insert(
            bssid.to_string(),
            crate::debounce::ScanEntry {
                rssi_source: crate::rssi::RssiSource::IwDbm,
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn debug_releases_debouncer_before_waiting_for_db_reader() {
        let state = test_state();
        let bssid = "b8:00:00:00:20:02";
        let mut sample = crate::debounce::ScanSample::new();
        sample.insert(
            bssid.to_string(),
            crate::debounce::ScanEntry {
                rssi_source: crate::rssi::RssiSource::IwDbm,
                signal_dbm: -48,
                ssid: Some("debug-lock-scope".to_string()),
                channel: Some(whereami_protocol::Channel::TwoFourGhz(11)),
            },
        );
        state.debouncer.lock().await.push_scan(sample);

        // Stall the real reader actor so debug_statuses cannot finish. One
        // populated BSSID ensures this covers the former per-BSSID DB-await
        // shape rather than the empty-debug shortcut.
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker_db = state.db.clone();
        let mut blocker =
            tokio::spawn(async move { blocker_db.hold_reader_until(entered_tx, release_rx).await });
        let reader_entered = entered_rx.recv_timeout(Duration::from_secs(2));

        // Poll debug once while the mutex is held here. That puts its lock
        // future at the front of Tokio Mutex's FIFO queue without relying on a
        // scheduler sleep. Once this guard drops, debug must acquire first.
        let debouncer = state.debouncer.lock().await;
        let debug_state = Arc::clone(&state);
        let mut debug = Box::pin(async move { handle_debug(&debug_state).await });
        let early_debug =
            std::future::poll_fn(|cx| match std::future::Future::poll(debug.as_mut(), cx) {
                std::task::Poll::Pending => std::task::Poll::Ready(None),
                std::task::Poll::Ready(response) => std::task::Poll::Ready(Some(response)),
            })
            .await;
        if let Some(response) = early_debug {
            drop(debouncer);
            let _ = release_tx.send(());
            if tokio::time::timeout(Duration::from_secs(2), &mut blocker)
                .await
                .is_err()
            {
                blocker.abort();
                let _ = blocker.await;
            }
            panic!("debug finished while the debouncer mutex was held: {response}");
        }
        let mut debug = tokio::spawn(debug);
        drop(debouncer);

        // Scan isolates the debouncer: unlike locate it has no DB dependency.
        // It can complete only if debug dropped the guard before its blocked
        // reader await. The timeout also makes the old lock scope fail fast.
        let scan = tokio::time::timeout(Duration::from_millis(500), handle_scan(&state)).await;
        let debug_still_waiting = !debug.is_finished();

        // Always release and join the actor blocker before making assertions
        // about the observed ordering, so a failure cannot strand DB threads.
        let release_result = release_tx.send(());
        let blocker_result = tokio::time::timeout(Duration::from_secs(2), &mut blocker).await;
        if blocker_result.is_err() {
            blocker.abort();
            let _ = blocker.await;
        }
        let debug_result = tokio::time::timeout(Duration::from_secs(2), &mut debug).await;
        if debug_result.is_err() {
            debug.abort();
            let _ = debug.await;
        }

        reader_entered.expect("reader blocker must enter within two seconds");
        release_result.expect("reader blocker must still be waiting for release");
        blocker_result
            .expect("reader blocker must finish after release")
            .expect("reader blocker task must not panic")
            .expect("reader blocker must succeed");
        assert!(
            debug_still_waiting,
            "debug must remain stalled on the occupied reader actor"
        );

        let scan = scan.expect("scan must not wait behind debug's DB lookup");
        let scan: serde_json::Value = serde_json::from_str(&scan).unwrap();
        assert_eq!(scan["ok"], true);
        assert_eq!(scan["networks"][0]["bssid"], bssid);

        let debug = debug_result
            .expect("debug must finish after reader release")
            .expect("debug task must not panic");
        let debug: serde_json::Value = serde_json::from_str(&debug).unwrap();
        assert_eq!(debug["ok"], true);
        assert_eq!(debug["bssids"][0]["bssid"], bssid);
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
            raw: false,
            max_fixes: None,
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

    fn raw_history_request(from: &str, to: &str, max_fixes: Option<u64>) -> Request {
        Request::History {
            range: None,
            from: Some(from.to_string()),
            to: Some(to.to_string()),
            raw: true,
            max_fixes,
        }
    }

    #[tokio::test]
    async fn raw_history_returns_short_visit_that_segmentation_discards() {
        let state = test_state();
        let base = chrono::DateTime::parse_from_rfc3339("2026-05-19T06:40:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        for (index, seconds) in [0, 40, 80, 120].into_iter().enumerate() {
            lock_db(&state)
                .insert_fix(
                    &(base + chrono::Duration::seconds(seconds)).to_rfc3339(),
                    55.74391,
                    12.57933,
                    90.0 - index as f64,
                    8 - index as i64,
                )
                .unwrap();
        }

        let raw = dispatch_command(
            &raw_history_request(
                "2026-05-19T06:40:00+00:00",
                "2026-05-19T06:42:00+00:00",
                Some(1_000),
            ),
            &state,
        )
        .await;
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["ok"], true, "raw history failed: {raw}");
        assert_eq!(
            value.as_object().unwrap().len(),
            4,
            "uncapped raw response must be exactly ok/v/range/fixes: {raw}"
        );
        assert_eq!(
            value["range"],
            serde_json::json!({
                "from_rfc3339": "2026-05-19T06:40:00+00:00",
                "to_rfc3339": "2026-05-19T06:42:00+00:00"
            })
        );
        let fixes = value["fixes"].as_array().unwrap();
        assert_eq!(fixes.len(), 4);
        assert!(value.get("elided").is_none());
        for (index, fix) in fixes.iter().enumerate() {
            assert_eq!(
                fix.as_object().unwrap().len(),
                5,
                "raw fix must expose no database id or internal fields"
            );
            assert_eq!(fix["lat"], 55.74391);
            assert_eq!(fix["lon"], 12.57933);
            assert_eq!(fix["accuracy_m"], 90.0 - index as f64);
            assert_eq!(fix["n_sources"], 8 - index as i64);
            assert_eq!(
                fix["at_rfc3339"],
                (base + chrono::Duration::seconds(index as i64 * 40)).to_rfc3339()
            );
        }
        assert!(value.get("from").is_none());
        assert!(value.get("to").is_none());
        assert!(value.get("segments").is_none());
        drop(raw);

        let segmented = dispatch_command(
            &Request::History {
                range: None,
                from: Some("2026-05-19T06:40:00+00:00".to_string()),
                to: Some("2026-05-19T06:42:00+00:00".to_string()),
                raw: false,
                max_fixes: None,
            },
            &state,
        )
        .await;
        let segmented_value: serde_json::Value = serde_json::from_str(&segmented).unwrap();
        assert_eq!(segmented_value["ok"], true);
        assert_eq!(
            segmented_value.as_object().unwrap().len(),
            5,
            "legacy response must remain exactly ok/v/from/to/segments"
        );
        assert_eq!(segmented_value["segments"].as_array().unwrap().len(), 0);
        assert!(segmented_value.get("range").is_none());
        assert!(segmented_value.get("fixes").is_none());
    }

    #[tokio::test]
    async fn task0245_raw_history_emits_known_quality_and_capped_projection() {
        let state = test_state();
        let cold = FixQuality::new(None, None, Some(1.0), Confidence::High, None).unwrap();
        state
            .db
            .persist_fix_and_history(
                LastFix {
                    lat: 55.0,
                    lon: 12.0,
                    accuracy_m: 10.0,
                    address: None,
                    at: chrono::DateTime::parse_from_rfc3339("2026-07-05T12:00:00+00:00")
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                    sources: 3,
                },
                cold,
            )
            .await
            .unwrap();
        let suspect = FixQuality::new(
            Some(321.5),
            Some(60),
            Some(0.75),
            Confidence::Suspect,
            Some("exact reason".to_owned()),
        )
        .unwrap();
        state
            .db
            .persist_fix_and_history(
                LastFix {
                    lat: 56.0,
                    lon: 13.0,
                    accuracy_m: 20.0,
                    address: None,
                    at: chrono::DateTime::parse_from_rfc3339("2026-07-05T12:01:00+00:00")
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                    sources: 4,
                },
                suspect,
            )
            .await
            .unwrap();

        let response = dispatch_command(
            &raw_history_request(
                "2026-07-05T12:00:00+00:00",
                "2026-07-05T12:01:00+00:00",
                Some(1),
            ),
            &state,
        )
        .await;
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["elided"], 1);
        let fixes = value["fixes"].as_array().unwrap();
        assert_eq!(fixes.len(), 1);
        assert_eq!(
            fixes[0],
            serde_json::json!({
                "at_rfc3339": "2026-07-05T12:01:00+00:00",
                "lat": 56.0,
                "lon": 13.0,
                "accuracy_m": 20.0,
                "n_sources": 4,
                "implied_speed_kmh": 321.5,
                "last_fix_age_secs": 60,
                "inlier_ratio": 0.75,
                "confidence": "suspect",
                "suspect_reason": "exact reason"
            })
        );
        drop(response);

        let uncapped = dispatch_command(
            &raw_history_request(
                "2026-07-05T12:00:00+00:00",
                "2026-07-05T12:01:00+00:00",
                Some(2),
            ),
            &state,
        )
        .await;
        let uncapped: serde_json::Value = serde_json::from_str(&uncapped).unwrap();
        let cold = &uncapped["fixes"][0];
        assert_eq!(cold["confidence"], "high");
        assert_eq!(cold["inlier_ratio"], 1.0);
        for absent in ["implied_speed_kmh", "last_fix_age_secs", "suspect_reason"] {
            assert!(cold.get(absent).is_none(), "{absent} must be omitted");
        }
    }

    #[tokio::test]
    async fn task0245_quality_metadata_does_not_change_segmented_history_bytes() {
        let legacy = test_state();
        let known = test_state();
        for (timestamp, lat, lon) in [
            ("2026-07-05T12:00:00+00:00", 55.0, 12.0),
            ("2026-07-05T12:10:00+00:00", 55.00001, 12.00001),
        ] {
            lock_db(&legacy)
                .insert_fix(timestamp, lat, lon, 10.0, 3)
                .unwrap();
            known
                .db
                .persist_fix_and_history(
                    LastFix {
                        lat,
                        lon,
                        accuracy_m: 10.0,
                        address: None,
                        at: chrono::DateTime::parse_from_rfc3339(timestamp)
                            .unwrap()
                            .with_timezone(&chrono::Utc),
                        sources: 3,
                    },
                    FixQuality::new(None, None, Some(1.0), Confidence::High, None).unwrap(),
                )
                .await
                .unwrap();
        }
        let request = Request::History {
            range: None,
            from: Some("2026-07-05T12:00:00+00:00".to_owned()),
            to: Some("2026-07-05T12:10:00+00:00".to_owned()),
            raw: false,
            max_fixes: None,
        };
        let legacy_response = dispatch_command(&request, &legacy).await;
        let known_response = dispatch_command(&request, &known).await;
        assert_eq!(&*known_response, &*legacy_response);
        let value: serde_json::Value = serde_json::from_str(&known_response).unwrap();
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["from", "ok", "segments", "to", "v"]
        );
    }

    #[tokio::test]
    async fn raw_history_default_cap_returns_newest_thousand_and_exact_elision() {
        let state = test_state();
        let base = chrono::DateTime::parse_from_rfc3339("2026-05-01T00:00:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        for index in 0..1_500 {
            lock_db(&state)
                .insert_fix(
                    &(base + chrono::Duration::seconds(index)).to_rfc3339(),
                    55.0 + index as f64 / 100_000.0,
                    12.0,
                    5.0,
                    index,
                )
                .unwrap();
        }
        let from = base.to_rfc3339();
        let to = (base + chrono::Duration::seconds(1_499)).to_rfc3339();

        let default_page = dispatch_command(&raw_history_request(&from, &to, None), &state).await;
        let value: serde_json::Value = serde_json::from_str(&default_page).unwrap();
        assert_eq!(value["ok"], true, "raw history failed: {default_page}");
        assert_eq!(value.as_object().unwrap().len(), 5);
        let fixes = value["fixes"].as_array().unwrap();
        assert_eq!(fixes.len(), 1_000);
        assert_eq!(fixes[0]["n_sources"], 500);
        assert_eq!(fixes[999]["n_sources"], 1_499);
        assert_eq!(value["elided"], 500);
        for (offset, fix) in fixes.iter().enumerate() {
            assert_eq!(fix["n_sources"], offset + 500);
        }
        drop(default_page);

        let exact_page =
            dispatch_command(&raw_history_request(&from, &to, Some(1_500)), &state).await;
        let exact: serde_json::Value = serde_json::from_str(&exact_page).unwrap();
        assert_eq!(exact.as_object().unwrap().len(), 4);
        assert_eq!(exact["fixes"].as_array().unwrap().len(), 1_500);
        assert!(exact.get("elided").is_none());
    }

    #[tokio::test]
    async fn history_raw_cap_validation_is_explicit() {
        let state = test_state();
        for request in [
            Request::History {
                range: Some("1h".to_string()),
                from: None,
                to: None,
                raw: false,
                max_fixes: Some(100),
            },
            Request::History {
                range: Some("1h".to_string()),
                from: None,
                to: None,
                raw: true,
                max_fixes: Some(0),
            },
            Request::History {
                range: Some("1h".to_string()),
                from: None,
                to: None,
                raw: true,
                max_fixes: Some(MAX_RAW_HISTORY_FIXES + 1),
            },
        ] {
            let response = dispatch_command(&request, &state).await;
            let value: serde_json::Value = serde_json::from_str(&response).unwrap();
            assert_eq!(value["ok"], false, "invalid cap succeeded: {response}");
            assert!(
                value["error"].as_str().unwrap().contains("max_fixes"),
                "{response}"
            );
        }
    }

    fn one_hour_history_request() -> Request {
        Request::History {
            range: Some("1h".to_string()),
            from: None,
            to: None,
            raw: false,
            max_fixes: None,
        }
    }

    fn one_hour_raw_history_request() -> Request {
        Request::History {
            range: Some("1h".to_string()),
            from: None,
            to: None,
            raw: true,
            max_fixes: None,
        }
    }

    #[tokio::test]
    async fn raw_and_segmented_history_share_single_flight_admission() {
        let state = test_state();
        let segmented = dispatch_command(&one_hour_history_request(), &state).await;
        let raw_busy = dispatch_command(&one_hour_raw_history_request(), &state).await;
        assert!(
            raw_busy.contains("history: busy; retry later"),
            "{raw_busy}"
        );
        drop(raw_busy);
        drop(segmented);

        let raw = dispatch_command(&one_hour_raw_history_request(), &state).await;
        let segmented_busy = dispatch_command(&one_hour_history_request(), &state).await;
        assert!(
            segmented_busy.contains("history: busy; retry later"),
            "{segmented_busy}"
        );
        drop(segmented_busy);
        drop(raw);
        assert_eq!(state.history_slot.available_permits(), 1);
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
                output: HistoryOutput::Segmented {
                    dist_threshold_m: 100.0,
                    min_duration_secs: 300,
                },
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
                output: HistoryOutput::Segmented {
                    dist_threshold_m: 100.0,
                    min_duration_secs: 300,
                },
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
                output: HistoryOutput::Segmented {
                    dist_threshold_m: 100.0,
                    min_duration_secs: 300,
                },
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
            provider_reported_lat: 55.0,
            provider_reported_lon: 12.0,
            encryption: None,
            frequency: None,
            city: None,
            country: None,
            is_mobile: false,
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
            provider_reported_lat: 55.0,
            provider_reported_lon: 12.0,
            encryption: None,
            frequency: None,
            city: None,
            country: None,
            is_mobile: false,
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
            provider_reported_lat: 55.0,
            provider_reported_lon: 12.0,
            encryption: None,
            frequency: None,
            city: None,
            country: None,
            is_mobile: false,
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
