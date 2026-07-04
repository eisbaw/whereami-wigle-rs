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
use crate::db::Database;
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
/// BSSIDs is ~7000 entries; resolve_chain takes the DB lock per BSSID,
/// so an unbounded count would let one hostile request starve the rest
/// of the daemon. task-0053.
const MAX_RESOLVE_BSSIDS: usize = 256;

// task-0056: AddressCache extracted to server/address_cache.rs. Public
// re-export so existing call sites (`crate::server::AddressCache`) keep
// working.
mod address_cache;
use address_cache::address_cache_key;
pub use address_cache::AddressCache;

/// Acquire the DB lock, logging a warning if the mutex was poisoned.
pub fn lock_db(state: &DaemonState) -> std::sync::MutexGuard<'_, crate::db::Database> {
    state.db.lock().unwrap_or_else(|e| {
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

    /// Run a closure against the locked Database. Centralises the
    /// poison-recovery + lock-scoping pattern so callers don't have to
    /// do `let db = lock_db(state); db.foo()` every time. For
    /// long-running access that spans multiple operations, use
    /// `lock_db` directly to keep the guard alive across statements.
    /// task-0078.
    pub fn with_db<R>(&self, f: impl FnOnce(&crate::db::Database) -> R) -> R {
        let db = self.db.lock().unwrap_or_else(|e| {
            tracing::error!(
                "DB mutex was poisoned (a thread panicked while holding it) — recovering"
            );
            e.into_inner()
        });
        f(&db)
    }

    /// Spawn a detached background task tracked by the shutdown drain
    /// (task-0102). The scan loop's proactive resolve, the locate handler's
    /// fallback resolve, and the Nominatim address backfill all route through
    /// here so a SIGTERM can wait for them under one budget and force-abort any
    /// stragglers instead of killing them mid-API-call at process exit.
    ///
    /// Refuses to spawn once shutdown has been signalled — this structurally
    /// enforces "no new work after shutdown began" (task-0190 AC#2); the future
    /// is dropped, not run. Completed tasks are reaped on each call so the
    /// JoinSet does not accumulate finished-task slots (a JoinSet only frees a
    /// slot when the task is joined).
    ///
    /// Per-connection handlers are deliberately NOT tracked here — they are
    /// bounded by READ/WRITE timeouts and owning them is task-0201's concern.
    pub fn spawn_tracked<F>(&self, fut: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        if self.shutdown.is_cancelled() {
            debug!("spawn_tracked refused: shutdown in progress");
            return;
        }
        let mut set = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        while set.try_join_next().is_some() {}
        set.spawn(fut);
    }
}

/// Shared daemon state, accessible from connection handlers.
/// Database uses std::sync::Mutex because rusqlite::Connection is !Send.
/// All DB ops are synchronous and fast, so we never hold the lock across await points.
pub struct DaemonState {
    pub db: std::sync::Mutex<Database>,
    pub debouncer: tokio::sync::Mutex<Debouncer>,
    pub args: Args,
    pub wigle: WigleClient,
    pub apple: AppleClient,
    pub nominatim: NominatimClient,
    pub last_fix: tokio::sync::Mutex<Option<LastFix>>,
    /// BSSIDs currently undergoing remote provider lookup. Used by
    /// `resolver::resolve_chain` to coalesce concurrent requests for the
    /// same BSSID (scan loop + locate cold-start + pending drain can all
    /// fire at once otherwise). std::sync::Mutex is fine: we only ever hold
    /// it for an insert or a remove, never across await points, and that
    /// makes Drop-based cleanup viable for future RAII guards.
    pub inflight: std::sync::Mutex<std::collections::HashSet<String>>,
    /// Reverse-geocoded street addresses keyed by rounded (lat, lon).
    /// Populated lazily in the background after a `locate` returns; the
    /// next call at the same rounded position gets the address for free
    /// without blocking on Nominatim's 1 req/s rate limit.
    pub address_cache: std::sync::Mutex<AddressCache>,
    /// Cumulative count of best-effort DB writes that failed (warn-only
    /// sites). Surfaced via the stats endpoint (task-0076) so silent
    /// corruption is visible without scraping the journal.
    pub db_write_failures: std::sync::atomic::AtomicU64,
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
    /// Detached background tasks (proactive `resolve_background` from the scan
    /// loop and the locate handler, plus the Nominatim address backfill) are
    /// tracked here so the shutdown drain can wait for them under one budget
    /// and force-abort any stragglers (task-0102). A `std::sync::Mutex` guards
    /// the `JoinSet` because it is not `Sync` for concurrent spawn; the guard
    /// is only ever held to spawn/reap/take, never across an await.
    pub tasks: std::sync::Mutex<tokio::task::JoinSet<()>>,
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
    /// exhausted quota), or a transient failure occurred. Distinct from
    /// NotFound so clients that cache not_found don't blacklist a BSSID that
    /// is still resolvable; the client should retry later.
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
    /// Number of BSSIDs currently undergoing remote provider lookup
    /// (size of state.inflight). task-0074.
    inflight: usize,
    /// Cumulative count of best-effort DB writes that failed and were
    /// warned-only. Helps spot silent corruption (full disk, missing
    /// write permissions, etc.). Reset on daemon restart. task-0076.
    db_write_failures: u64,
    /// Number of background tasks (tracked resolve/backfill spawns and the
    /// named scan/drain/prune/server loops) force-aborted at the end of the
    /// LAST clean shutdown because they did not finish within the drain
    /// budget. Persisted at shutdown and reloaded at startup — the count is
    /// produced after the server socket closes, so the previous run's value
    /// is the only observable one. A crash/SIGKILL does not update it.
    /// task-0102.
    orphan_tasks_aborted: u64,
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
/// startup with a non-zero exit. TCP is opt-in and bound HERE so its failure
/// flows through the existing task-0195 server-task self-exit path (clean
/// exit) rather than aborting daemon startup. task-0196.
pub async fn run_server(state: Arc<DaemonState>, unix: UnixListener) -> Result<()> {
    let tcp: Option<TcpListener> = match state.args.parsed_bind()? {
        Some(addr) => {
            let listener = TcpListener::bind(addr).await?;
            let bound = listener.local_addr().unwrap_or(addr);
            if bound.ip().is_loopback() {
                warn!(
                    "TCP listening on {bound}: this transport has NO peer authentication — every \
                     local user can read location history, live position and Wi-Fi scans and \
                     drive WiGLE quota. The unix socket is the access-controlled transport."
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
        None => None,
    };
    info!("control channel: unix socket ready");

    loop {
        tokio::select! {
            r = unix.accept() => match r {
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
                    spawn_handler(stream, Arc::clone(&state));
                }
                Err(e) => error!("unix accept error: {e}"),
            },
            r = async {
                match &tcp {
                    Some(l) => l.accept().await,
                    // No TCP configured: never resolve, so this branch is inert
                    // and the unix branch drives the loop (task-0196).
                    None => std::future::pending().await,
                }
            } => match r {
                Ok((stream, addr)) => {
                    debug!("tcp connection from {addr}");
                    spawn_handler(stream, Arc::clone(&state));
                }
                Err(e) => error!("tcp accept error: {e}"),
            },
        }
    }
}

/// Spawn a detached per-connection handler generic over the stream type.
/// Detached (NOT spawn_tracked): each is bounded by READ_TIMEOUT/WRITE_TIMEOUT,
/// and tracking them across shutdown is task-0201's scope (task-0102).
fn spawn_handler<S>(stream: S, state: Arc<DaemonState>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        if let Err(e) = handle_connection(stream, state).await {
            warn!("connection error: {e}");
        }
    });
}

/// Handle one request/response exchange. Generic over the transport so the unix
/// socket and opt-in TCP share identical framing, limits and timeouts
/// (task-0196). `tokio::io::split` works for any `AsyncRead + AsyncWrite`,
/// unlike the type-specific `into_split`.
async fn handle_connection<S>(stream: S, state: Arc<DaemonState>) -> Result<()>
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

    let response = match serde_json::from_str::<Request>(line.trim()) {
        Ok(req) => dispatch_command(&req, &state).await,
        Err(e) => error_json(&format!("invalid request: {e}")),
    };

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

async fn dispatch_command(req: &Request, state: &Arc<DaemonState>) -> String {
    match req {
        Request::Locate => handle_locate(state).await,
        Request::Resolve { bssids } => handle_resolve(bssids, state).await,
        Request::Scan => handle_scan(state).await,
        Request::Stats => handle_stats(state).await,
        Request::Debug => handle_debug(state).await,
        Request::Version => handle_version(),
        Request::History { range, from, to } => {
            handle_history(range.as_deref(), from.as_deref(), to.as_deref(), state).await
        }
    }
}

async fn handle_locate(state: &Arc<DaemonState>) -> String {
    use crate::resolver;
    use crate::trilaterate::{trilaterate, PositionedAp};

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

    // Try stable candidates first, fall back to raw visible APs
    let using_fallback = candidates.is_empty() && !fallback_candidates.is_empty();
    let active_candidates = if candidates.is_empty() {
        &fallback_candidates
    } else {
        &candidates
    };

    if active_candidates.is_empty() {
        return return_last_fix_or_error(state, "no APs detected yet", visible, stable_count).await;
    }

    let bssids: Vec<String> = active_candidates.iter().map(|(b, _)| b.clone()).collect();
    let signal_map: std::collections::HashMap<String, i32> =
        active_candidates.iter().cloned().collect();

    // Cache-only lookup — never blocks on WiGLE. Instant response.
    let cached_aps = resolver::lookup_cached(&bssids, state);
    let cached_count = cached_aps.len();

    if cached_aps.is_empty() {
        // Nothing cached. If using fallback, queue all for background resolution.
        if using_fallback {
            // task-0102: track so the shutdown drain waits for this resolve
            // instead of killing it mid-API-call (which would leak a WiGLE
            // quota slot / leave partial writes).
            let resolve_state = Arc::clone(state);
            let bssids_clone = bssids.clone();
            state.spawn_tracked(async move {
                resolver::resolve_background(&bssids_clone, &resolve_state).await;
            });
        }
        return return_last_fix_or_error(
            state,
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
            // Address resolution is decoupled from the locate response.
            //
            // Hot path: probe the in-memory address cache by rounded
            // (lat, lon). Hit -> attach immediately. Miss -> attach
            // None and spawn a background task that calls Nominatim;
            // the next locate at this rounded position will hit.
            //
            // This keeps the locate latency at trilateration time
            // (~ms) instead of Nominatim time (~1s, with a 1 req/s
            // rate-limit mutex that previously also serialised
            // concurrent locate calls).
            let address = if state.args.address_approx {
                let cached = {
                    let cache = state.address_cache.lock().unwrap_or_else(|e| {
                        warn!("address cache mutex poisoned — recovering");
                        e.into_inner()
                    });
                    cache.get(pos.lat, pos.lon)
                };
                if cached.is_none() {
                    // task-0102: track the backfill so shutdown drains it
                    // rather than aborting it mid-Nominatim-call.
                    let bg_state = Arc::clone(state);
                    let bg_lat = pos.lat;
                    let bg_lon = pos.lon;
                    state.spawn_tracked(async move {
                        match bg_state.nominatim.reverse_geocode(bg_lat, bg_lon).await {
                            Ok(addr) => {
                                let display = addr.display;
                                {
                                    let mut cache =
                                        bg_state.address_cache.lock().unwrap_or_else(|e| {
                                            warn!(
                                                "address cache mutex poisoned on insert \
                                                 — recovering"
                                            );
                                            e.into_inner()
                                        });
                                    cache.insert(bg_lat, bg_lon, display.clone());
                                }
                                // Backfill last_fix.address if it still
                                // matches the position we just resolved.
                                // If a newer fix has overwritten it we
                                // leave it alone (the newer one will
                                // resolve its own address).
                                //
                                // The DB write happens *while we still hold*
                                // the in-memory last_fix mutex (task-0046),
                                // so a concurrent handle_locate cannot
                                // replace the in-memory row, persist its own
                                // newer row, and then have us overwrite that
                                // newer row with stale lat/lon. Ordering is
                                // last_fix (tokio) -> db (std), matching
                                // handle_locate.
                                let mut last = bg_state.last_fix.lock().await;
                                if let Some(fix) = last.as_mut() {
                                    // Fail-closed after task-0106 made
                                    // address_cache_key return Option. In this
                                    // caller bg_lat/bg_lon come from trilaterate
                                    // (routed through finite_position), so their
                                    // key is always Some; only fix.lat/lon can
                                    // be non-finite (a poisoned last_fix row
                                    // rehydrated from the DB). Requiring BOTH
                                    // keys present and equal is a strict
                                    // fail-closed guard: a non-finite fix (key
                                    // None) can never backfill an address onto a
                                    // garbage position.
                                    let keys_match = address_cache_key(fix.lat, fix.lon)
                                        .zip(address_cache_key(bg_lat, bg_lon))
                                        .is_some_and(|(a, b)| a == b);
                                    if keys_match && fix.address.is_none() {
                                        fix.address = Some(display.clone());
                                        let row: crate::db::LastFixRow = (&*fix).into();
                                        let db = lock_db(&bg_state);
                                        if let Err(e) = db.set_last_fix(&row) {
                                            warn!(
                                                "failed to persist last_fix address backfill: {e}"
                                            );
                                            bg_state.record_db_failure();
                                        }
                                    }
                                }
                            }
                            Err(e) => warn!("background reverse geocode failed: {e}"),
                        }
                    });
                }
                cached
            } else {
                None
            };

            // Save as last-known fix: in-memory + SQLite, both under the
            // last_fix mutex (task-0046).
            //
            // Holding the last_fix mutex across the DB write closes the race
            // where two concurrent handle_locate calls could otherwise have
            // their in-memory writes interleave with their DB writes:
            //   T1 in-mem X, T1 drops, T2 in-mem Y, T2 drops + persists Y,
            //   T1 persists X => disk diverges from in-memory.
            //
            // The DB writes are sync (no .await), so holding tokio::Mutex ->
            // std::Mutex briefly is correct. Order matches the address-
            // backfill task to avoid deadlock.
            //
            // History insert (task-0031) shares the same critical section
            // because the cost of a second SQL statement is negligible vs
            // the cost of duplicating the lock-scoping logic.
            let at = chrono::Utc::now();
            {
                let mut last = state.last_fix.lock().await;
                let new_fix = LastFix {
                    lat: pos.lat,
                    lon: pos.lon,
                    accuracy_m: pos.accuracy_m,
                    address: address.clone(),
                    at,
                    sources: cached_count,
                };
                let row: crate::db::LastFixRow = (&new_fix).into();
                *last = Some(new_fix);
                let db = lock_db(state);
                if let Err(e) = db.set_last_fix(&row) {
                    warn!("failed to persist last_fix: {e}");
                    state.record_db_failure();
                }
                if let Err(e) = db.insert_fix(
                    &at.to_rfc3339(),
                    pos.lat,
                    pos.lon,
                    pos.accuracy_m,
                    cached_count as i64,
                ) {
                    warn!("failed to record fix in history: {e}");
                    state.record_db_failure();
                }
            }

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
    error_msg: &str,
    visible: usize,
    stable: usize,
) -> String {
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

async fn handle_resolve(bssids: &[String], state: &Arc<DaemonState>) -> String {
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

    let result = resolver::resolve_readonly(&normalized, state).await;

    let results: Vec<ResolveResult> = normalized
        .iter()
        .map(|bssid| {
            if let Some(ap) = result.positioned.iter().find(|a| &a.bssid == bssid) {
                ResolveResult {
                    bssid: bssid.clone(),
                    lat: Some(ap.lat),
                    lon: Some(ap.lon),
                    ssid: ap.ssid.clone(),
                    provenance: if result.fetched_bssids.contains(bssid) {
                        Provenance::Api
                    } else {
                        Provenance::Cache
                    },
                }
            } else {
                // task-0194: only report not_found when the resolver
                // established *authoritative* absence for this call (a provider
                // said NotFound, or the not_found table did within TTL).
                // Everything else — deferred to an in-flight resolution, a
                // provider skipped for missing creds / exhausted quota, or a
                // transient failure — is `unresolved`: absence is NOT
                // established, so a client must retry rather than blacklist it.
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

    let mut bssids: Vec<DebugBssid> = all_bssids
        .iter()
        .map(|(bssid, &signal)| {
            let seen = debouncer.count(bssid);
            let is_stable = stable_set.contains(bssid);
            let db = lock_db(state);
            let db_status = if db.get_ap(bssid).ok().flatten().is_some() {
                DbStatus::Cached
            } else if db
                .is_not_found(bssid, state.args.not_found_ttl_days)
                .unwrap_or(false)
            {
                DbStatus::NotFound
            } else if db.is_pending(bssid).unwrap_or(false) {
                DbStatus::Pending
            } else {
                DbStatus::New
            };
            DebugBssid {
                bssid: bssid.clone(),
                signal_dbm: signal,
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

    drop(debouncer);

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

async fn handle_history(
    range: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    state: &Arc<DaemonState>,
) -> String {
    // Resolve range: either `range="7d"` shorthand OR explicit from/to RFC3339.
    // Mutually exclusive. If neither is provided, default to the last 7 days.
    let (from, to) = match (range, from, to) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            return error_json("history: 'range' and 'from'/'to' are mutually exclusive");
        }
        (Some(spec), None, None) => match crate::history::parse_range(spec) {
            Ok(r) => r,
            Err(e) => return error_json(&format!("history: invalid range: {e}")),
        },
        (None, Some(f), Some(t)) => {
            let f = match chrono::DateTime::parse_from_rfc3339(f) {
                Ok(d) => d.with_timezone(&chrono::Utc),
                Err(e) => return error_json(&format!("history: invalid 'from' timestamp: {e}")),
            };
            let t = match chrono::DateTime::parse_from_rfc3339(t) {
                Ok(d) => d.with_timezone(&chrono::Utc),
                Err(e) => return error_json(&format!("history: invalid 'to' timestamp: {e}")),
            };
            if f >= t {
                return error_json("history: 'from' must be before 'to'");
            }
            (f, t)
        }
        (None, None, None) => {
            crate::history::parse_range("7d").expect("hardcoded default range '7d' must parse")
        }
        _ => return error_json("history: provide either 'range' or both 'from' and 'to'"),
    };

    let fixes = {
        let db = lock_db(state);
        match db.get_fixes_in_range(&from.to_rfc3339(), &to.to_rfc3339()) {
            Ok(v) => v,
            Err(e) => return error_json(&format!("history: db error: {e}")),
        }
    };

    let segments = crate::history::segment_fixes(
        &fixes,
        state.args.history_segment_distance_m as f64,
        state.args.history_segment_min_duration_secs as i64,
    );

    let resp = HistoryResponse {
        ok: true,
        v: 1,
        from: from.to_rfc3339(),
        to: to.to_rfc3339(),
        segments,
    };
    ok_json(&resp)
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
    let orphan_tasks_aborted = state
        .orphan_tasks_aborted
        .load(std::sync::atomic::Ordering::Relaxed);
    // task-0078: with_db scopes the lock + poison recovery. Each
    // .unwrap_or(0) here logs nothing on failure — see db_write_failures
    // (task-0076) for cumulative visibility on silent write failures.
    let resp = state.with_db(|db| StatsResponse {
        ok: true,
        v: 1,
        cached_aps: db.cached_ap_count().unwrap_or(0),
        pending_aps: db.pending_ap_count().unwrap_or(0),
        not_found_aps: db.not_found_ap_count().unwrap_or(0),
        db_size_bytes: db.db_size_bytes().unwrap_or(0),
        api_calls_today: db.api_calls_today().unwrap_or(0),
        inflight,
        db_write_failures,
        orphan_tasks_aborted,
    });
    ok_json(&resp)
}

// Address-cache tests moved to server/address_cache.rs alongside the impl.

/// Build a minimal, side-effect-free `DaemonState` backed by an in-memory
/// SQLite DB, shared by the in-file test modules. The network clients construct
/// HTTP clients only (no I/O at construction), so this is cheap and hermetic.
#[cfg(test)]
pub(crate) fn test_state() -> std::sync::Arc<DaemonState> {
    use clap::Parser;
    let args = crate::config::Args::parse_from(["whereamid"]);
    std::sync::Arc::new(DaemonState {
        db: std::sync::Mutex::new(crate::db::Database::open_memory().unwrap()),
        debouncer: tokio::sync::Mutex::new(crate::debounce::Debouncer::new(
            args.debounce_window,
            args.debounce_threshold,
        )),
        wigle: crate::wigle::WigleClient::new("", ""),
        apple: crate::apple::AppleClient::new(),
        nominatim: crate::nominatim::NominatimClient::new(),
        last_fix: tokio::sync::Mutex::new(None),
        inflight: std::sync::Mutex::new(std::collections::HashSet::new()),
        address_cache: std::sync::Mutex::new(AddressCache::new()),
        db_write_failures: std::sync::atomic::AtomicU64::new(0),
        shutdown: tokio_util::sync::CancellationToken::new(),
        tasks: std::sync::Mutex::new(tokio::task::JoinSet::new()),
        orphan_tasks_aborted: std::sync::atomic::AtomicU64::new(0),
        args,
    })
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
    //! (task-0184) and resolve provenance mapping (task-0194).
    use super::test_state;
    use super::*;

    /// Extract `results[0].source` from a resolve dispatch response.
    fn resolve_source(resp: &str) -> String {
        let v: serde_json::Value =
            serde_json::from_str(resp).expect("dispatch must return valid JSON");
        assert_eq!(
            v.get("ok").and_then(|b| b.as_bool()),
            Some(true),
            "resolve must succeed, got {resp}"
        );
        v["results"][0]["source"]
            .as_str()
            .unwrap_or_else(|| panic!("results[0].source missing in {resp}"))
            .to_string()
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
        state.inflight.lock().unwrap().insert(bssid.clone());

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
        let dir = std::env::temp_dir().join(format!(
            "whereamid-xport-{tag}-{}-{}",
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
            handle_connection(stream, state).await.unwrap();
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
}
