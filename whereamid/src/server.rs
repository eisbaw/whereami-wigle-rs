//! TCP server: JSON-lines protocol, one-shot connections.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
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

#[derive(Serialize)]
struct ResolveResult {
    bssid: String,
    lat: Option<f64>,
    lon: Option<f64>,
    ssid: Option<String>,
    source: String,
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

/// Start the TCP server. Runs until cancelled.
pub async fn run_server(state: Arc<DaemonState>) -> Result<()> {
    let listener = TcpListener::bind(&state.args.bind).await?;
    info!("listening on {}", state.args.bind);

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                debug!("connection from {addr}");
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, state).await {
                        warn!("connection error: {e}");
                    }
                });
            }
            Err(e) => {
                error!("accept error: {e}");
            }
        }
    }
}

async fn handle_connection(stream: TcpStream, state: Arc<DaemonState>) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
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
            let resolve_state = Arc::clone(state);
            let bssids_clone = bssids.clone();
            tokio::spawn(async move {
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

    // Build trilateration input
    let positioned_aps: Vec<PositionedAp> = cached_aps
        .iter()
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
                    let bg_state = Arc::clone(state);
                    let bg_lat = pos.lat;
                    let bg_lon = pos.lon;
                    tokio::spawn(async move {
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
                                    if address_cache_key(fix.lat, fix.lon)
                                        == address_cache_key(bg_lat, bg_lon)
                                        && fix.address.is_none()
                                    {
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
                    source: if result.fetched_bssids.contains(bssid) {
                        "api".to_string()
                    } else {
                        "cache".to_string()
                    },
                }
            } else {
                ResolveResult {
                    bssid: bssid.clone(),
                    lat: None,
                    lon: None,
                    ssid: None,
                    source: "not_found".to_string(),
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

#[derive(Serialize)]
struct DebugBssid {
    bssid: String,
    /// None when the BSSID is debounce-stable but absent from the most
    /// recent scan (was previously fabricated as -90 dBm; task-0051).
    signal_dbm: Option<i32>,
    seen: usize,   // times seen in debounce window
    needed: usize, // threshold to be stable
    is_stable: bool,
    db_status: String, // "cached", "pending", "not_found", "new"
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
                "cached"
            } else if db
                .is_not_found(bssid, state.args.not_found_ttl_days)
                .unwrap_or(false)
            {
                "not_found"
            } else if db.is_pending(bssid).unwrap_or(false) {
                "pending"
            } else {
                "new"
            };
            DebugBssid {
                bssid: bssid.clone(),
                signal_dbm: signal,
                seen,
                needed: threshold,
                is_stable,
                db_status: db_status.to_string(),
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
    });
    ok_json(&resp)
}

// Address-cache tests moved to server/address_cache.rs alongside the impl.
