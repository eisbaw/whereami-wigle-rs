//! TCP server: JSON-lines protocol, one-shot connections.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

use crate::apple::AppleClient;
use crate::beacondb::BeaconDbClient;
use crate::config::Args;
use crate::db::Database;
use crate::debounce::Debouncer;
use crate::nominatim::NominatimClient;
use crate::wigle::WigleClient;

const READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST_BYTES: u64 = 64 * 1024; // 64 KiB max request size

/// Acquire the DB lock, logging a warning if the mutex was poisoned.
pub fn lock_db(state: &DaemonState) -> std::sync::MutexGuard<'_, crate::db::Database> {
    state.db.lock().unwrap_or_else(|e| {
        tracing::error!("DB mutex was poisoned (a thread panicked while holding it) — recovering");
        e.into_inner()
    })
}

/// Shared daemon state, accessible from connection handlers.
/// Database uses std::sync::Mutex because rusqlite::Connection is !Send.
/// All DB ops are synchronous and fast, so we never hold the lock across await points.
pub struct DaemonState {
    pub db: std::sync::Mutex<Database>,
    pub debouncer: tokio::sync::Mutex<Debouncer>,
    pub args: Args,
    pub wigle: WigleClient,
    #[allow(dead_code)] // kept for future BeaconDB integration
    pub beacondb: BeaconDbClient,
    pub apple: AppleClient,
    pub nominatim: NominatimClient,
}

// --- Protocol types ---

#[derive(Deserialize, Debug)]
struct Request {
    cmd: String,
    #[serde(default)]
    bssids: Vec<String>,
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
}

fn error_json(msg: &str) -> String {
    serde_json::to_string(&ErrorResponse {
        ok: false,
        v: 1,
        error: msg.to_string(),
    })
    .unwrap_or_else(|_| r#"{"ok":false,"v":1,"error":"serialization failed"}"#.to_string())
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

    writer.write_all(response.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.shutdown().await?;

    Ok(())
}

async fn dispatch_command(req: &Request, state: &Arc<DaemonState>) -> String {
    match req.cmd.as_str() {
        "locate" => handle_locate(state).await,
        "resolve" => handle_resolve(&req.bssids, state).await,
        "scan" => handle_scan(state).await,
        "stats" => handle_stats(state).await,
        "debug" => handle_debug(state).await,
        other => error_json(&format!("unknown command: {other}")),
    }
}

async fn handle_locate(state: &Arc<DaemonState>) -> String {
    use crate::resolver;
    use crate::trilaterate::{trilaterate, PositionedAp};

    let debouncer = state.debouncer.lock().await;
    let stable = debouncer.stable_bssids();
    let visible = debouncer.latest_scan().map(|s| s.len()).unwrap_or(0);

    // Build candidate list: stable APs with signal, sorted by RSSI, top-N
    let stable_count = stable.len();
    let mut candidates: Vec<(String, i32)> = stable
        .iter()
        .map(|b| {
            let signal = debouncer.latest_signal(b).unwrap_or(-90);
            (b.clone(), signal)
        })
        .collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1));
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
                v.sort_by(|a, b| b.1.cmp(&a.1));
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
        return error_json("no APs detected yet");
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
        let db = lock_db(state);
        let pending_count = db.pending_ap_count().unwrap_or(0);
        let cached_count = db.cached_ap_count().unwrap_or(0);
        drop(db);
        return error_json(&format!(
            "no cached positions for {} visible APs (db has {} cached, {} pending — resolving in background)",
            bssids.len(), cached_count, pending_count
        ));
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
            let address = if state.args.address_approx {
                match state.nominatim.reverse_geocode(pos.lat, pos.lon).await {
                    Ok(addr) => Some(addr.display),
                    Err(e) => {
                        warn!("reverse geocode failed: {e}");
                        None
                    }
                }
            } else {
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
            };
            serde_json::to_string(&resp).unwrap_or_else(|_| error_json("serialization failed"))
        }
        Err(e) => error_json(&format!("trilateration failed: {e}")),
    }
}

async fn handle_resolve(bssids: &[String], state: &Arc<DaemonState>) -> String {
    use crate::resolver;

    if bssids.is_empty() {
        return error_json("resolve requires non-empty bssids array");
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
    serde_json::to_string(&resp).unwrap_or_else(|_| error_json("serialization failed"))
}

async fn handle_scan(state: &Arc<DaemonState>) -> String {
    let debouncer = state.debouncer.lock().await;
    let scan_age_ms = debouncer.latest_scan_age_ms();
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
            nets.sort_by(|a, b| b.signal_dbm.cmp(&a.signal_dbm));
            nets
        }
        None => Vec::new(),
    };

    let resp = ScanResponse {
        ok: true,
        v: 1,
        networks,
        scan_age_ms,
    };
    serde_json::to_string(&resp).unwrap_or_else(|_| error_json("serialization failed"))
}

#[derive(Serialize)]
struct DebugBssid {
    bssid: String,
    signal_dbm: i32,
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

    // Collect ALL BSSIDs ever seen in the ring buffer
    let mut all_bssids: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    if let Some(scan) = debouncer.latest_scan() {
        for (bssid, entry) in scan {
            all_bssids.insert(bssid.clone(), entry.signal_dbm);
        }
    }
    // Also include stable ones not in latest scan
    for b in &stable_set {
        all_bssids.entry(b.clone()).or_insert(-90);
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
            } else if db
                .get_pending(1000)
                .unwrap_or_default()
                .iter()
                .any(|p| p.bssid == *bssid)
            {
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
    bssids.sort_by(|a, b| b.signal_dbm.cmp(&a.signal_dbm));

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
    serde_json::to_string(&resp).unwrap_or_else(|_| error_json("serialization failed"))
}

async fn handle_stats(state: &Arc<DaemonState>) -> String {
    let db = lock_db(state);
    let cached = db.cached_ap_count().unwrap_or(0);
    let pending = db.pending_ap_count().unwrap_or(0);
    let not_found = db.not_found_ap_count().unwrap_or(0);
    let db_size = db.db_size_bytes().unwrap_or(0);
    let api_calls = db.api_calls_today().unwrap_or(0);

    let resp = StatsResponse {
        ok: true,
        v: 1,
        cached_aps: cached,
        pending_aps: pending,
        not_found_aps: not_found,
        db_size_bytes: db_size,
        api_calls_today: api_calls,
    };
    serde_json::to_string(&resp).unwrap_or_else(|_| error_json("serialization failed"))
}
