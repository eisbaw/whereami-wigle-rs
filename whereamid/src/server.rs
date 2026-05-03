//! TCP server: JSON-lines protocol, one-shot connections.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

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
        other => error_json(&format!("unknown command: {other}")),
    }
}

async fn handle_locate(state: &Arc<DaemonState>) -> String {
    use crate::resolver;
    use crate::trilaterate::{trilaterate, PositionedAp};

    let debouncer = state.debouncer.lock().await;
    let stable = debouncer.stable_bssids();
    let visible = debouncer.latest_scan().map(|s| s.len()).unwrap_or(0);

    if stable.is_empty() {
        return error_json("no stable APs detected yet (debounce warming up)");
    }

    // Get top-N by signal strength. Use latest scan signal if available,
    // otherwise use a weak default (-90 dBm) so stable BSSIDs not in the
    // latest scan can still contribute from cache (just at lower weight).
    let stable_count = stable.len(); // PRD: count before top-N truncation
    let mut stable_with_signal: Vec<(String, i32)> = stable
        .iter()
        .map(|b| {
            let signal = debouncer.latest_signal(b).unwrap_or(-90);
            (b.clone(), signal)
        })
        .collect();
    stable_with_signal.sort_by(|a, b| b.1.cmp(&a.1));
    stable_with_signal.truncate(state.args.top_n);

    drop(debouncer); // Release lock before doing IO

    let bssids: Vec<String> = stable_with_signal.iter().map(|(b, _)| b.clone()).collect();
    let signal_map: std::collections::HashMap<String, i32> =
        stable_with_signal.into_iter().collect();

    // Resolve positions
    let resolve_result = resolver::resolve_for_locate(&bssids, state).await;

    if resolve_result.positioned.is_empty() {
        return error_json("no APs with known positions");
    }

    // Build trilateration input
    let positioned_aps: Vec<PositionedAp> = resolve_result
        .positioned
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
                sources: resolve_result.positioned.len(),
                cached: resolve_result.cached_count,
                fetched: resolve_result.fetched_count,
                pending: resolve_result.pending_count,
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
