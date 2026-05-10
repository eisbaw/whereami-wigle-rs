//! whereami-client: thin TCP client for the whereamid daemon.
//!
//! Connect to the daemon, send a JSON command, read a JSON response.
//! Each method opens a new TCP connection (one-shot protocol).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

/// Client for the whereamid daemon.
pub struct WhereAmIClient {
    addr: String,
}

/// Common shape of every daemon response (ok flag + optional error). Used
/// by CLI dispatchers to handle the ok/error/json triage uniformly
/// (task-0058).
pub trait DaemonResponse {
    fn is_ok(&self) -> bool;
    fn error(&self) -> Option<&str>;
}

// --- Request types ---

#[derive(Serialize)]
struct LocateRequest {
    cmd: &'static str,
}

#[derive(Serialize)]
struct ResolveRequest {
    cmd: &'static str,
    bssids: Vec<String>,
}

#[derive(Serialize)]
struct SimpleRequest {
    cmd: &'static str,
}

// --- Response types ---

/// Response from the `locate` command.
#[derive(Serialize, Deserialize, Debug)]
pub struct LocateResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub lat: f64,
    #[serde(default)]
    pub lon: f64,
    #[serde(default)]
    pub accuracy_m: f64,
    #[serde(default)]
    pub sources: usize,
    #[serde(default)]
    pub cached: usize,
    #[serde(default)]
    pub fetched: usize,
    #[serde(default)]
    pub pending: usize,
    #[serde(default)]
    pub visible: usize,
    #[serde(default)]
    pub stable: usize,
    #[serde(default)]
    pub address: Option<String>,
    /// True when the daemon could not produce a current fix and is
    /// returning the previous known position. Defaults to false.
    #[serde(default)]
    pub stale: bool,
    /// Age of the stale fix, in seconds. Only meaningful when `stale`.
    #[serde(default)]
    pub age_s: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A single result from the `resolve` command.
#[derive(Deserialize, Debug)]
pub struct ResolveResultEntry {
    pub bssid: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub ssid: Option<String>,
    pub source: String,
}

/// Response from the `resolve` command.
#[derive(Deserialize, Debug)]
pub struct ResolveResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub results: Vec<ResolveResultEntry>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A single network from the `scan` command.
#[derive(Serialize, Deserialize, Debug)]
pub struct NetworkEntry {
    pub bssid: String,
    pub ssid: Option<String>,
    pub signal_dbm: i32,
    pub channel: Option<i32>,
}

/// Response from the `scan` command.
#[derive(Serialize, Deserialize, Debug)]
pub struct ScanResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub networks: Vec<NetworkEntry>,
    /// Age of the most recent scan in milliseconds. None if no scan
    /// has completed yet.
    #[serde(default)]
    pub scan_age_ms: Option<u64>,
    /// RFC 3339 timestamp of the most recent scan, if any.
    #[serde(default)]
    pub scanned_at: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// One BSSID's debug state, as the daemon returns it under `bssids`.
#[derive(Serialize, Deserialize, Debug)]
pub struct DebugBssid {
    pub bssid: String,
    /// None when the BSSID is debounce-stable but missing from the most
    /// recent scan. Daemon previously fabricated -90 dBm here (task-0051).
    #[serde(default)]
    pub signal_dbm: Option<i32>,
    pub seen: usize,
    pub needed: usize,
    pub is_stable: bool,
    pub db_status: String,
}

/// Response from the `debug` command.
#[derive(Serialize, Deserialize, Debug)]
pub struct DebugResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub daemon_rev: Option<String>,
    #[serde(default)]
    pub scan_age_ms: Option<u64>,
    #[serde(default)]
    pub samples_in_buffer: usize,
    #[serde(default)]
    pub visible: usize,
    #[serde(default)]
    pub stable: usize,
    #[serde(default)]
    pub bssids: Vec<DebugBssid>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Response from the `version` command.
#[derive(Serialize, Deserialize, Debug)]
pub struct VersionResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub git_rev: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// One stay-point segment in a `history` response.
#[derive(Serialize, Deserialize, Debug)]
pub struct HistorySegment {
    pub start_rfc3339: String,
    pub end_rfc3339: String,
    pub duration_secs: i64,
    pub centroid_lat: f64,
    pub centroid_lon: f64,
    pub mean_accuracy_m: f64,
    pub n_fixes: usize,
}

/// Response from the `history` command.
#[derive(Serialize, Deserialize, Debug)]
pub struct HistoryResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub segments: Vec<HistorySegment>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Serialize)]
struct HistoryRequest {
    cmd: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    range: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<String>,
}

/// Response from the `stats` command.
#[derive(Serialize, Deserialize, Debug)]
pub struct StatsResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub cached_aps: i64,
    #[serde(default)]
    pub pending_aps: i64,
    #[serde(default)]
    pub not_found_aps: i64,
    #[serde(default)]
    pub db_size_bytes: i64,
    #[serde(default)]
    pub api_calls_today: u32,
    #[serde(default)]
    pub error: Option<String>,
}

impl WhereAmIClient {
    /// Create a new client connecting to the given address (e.g. "127.0.0.1:4747").
    pub fn new(addr: &str) -> Self {
        Self {
            addr: addr.to_string(),
        }
    }

    /// Create a client with the default address (127.0.0.1:4747).
    pub fn default_addr() -> Self {
        Self::new("127.0.0.1:4747")
    }

    /// Send a raw JSON command and return the raw response string.
    pub fn raw_command(&self, json: &str) -> Result<String> {
        let mut stream = TcpStream::connect(&self.addr)
            .with_context(|| format!("connecting to whereamid at {}", self.addr))?;
        stream
            .write_all(json.as_bytes())
            .context("sending request")?;
        stream.write_all(b"\n").context("sending newline")?;
        stream.flush().context("flushing")?;
        stream
            .shutdown(std::net::Shutdown::Write)
            .context("shutdown write")?;
        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader
            .read_line(&mut response_line)
            .context("reading response")?;
        Ok(response_line.trim().to_string())
    }

    /// Send a request and read the response. One-shot TCP connection.
    fn request<T: for<'de> Deserialize<'de>>(&self, json: &str) -> Result<T> {
        let mut stream = TcpStream::connect(&self.addr)
            .with_context(|| format!("connecting to whereamid at {}", self.addr))?;

        stream
            .write_all(json.as_bytes())
            .context("sending request")?;
        stream.write_all(b"\n").context("sending newline")?;
        stream.flush().context("flushing")?;

        // Signal we're done writing
        stream
            .shutdown(std::net::Shutdown::Write)
            .context("shutdown write")?;

        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader
            .read_line(&mut response_line)
            .context("reading response")?;

        if response_line.is_empty() {
            bail!("empty response from daemon");
        }

        let resp: T = serde_json::from_str(&response_line).context("parsing response JSON")?;
        Ok(resp)
    }

    /// Ask "where am I?" based on current stable APs.
    pub fn locate(&self) -> Result<LocateResponse> {
        let req = serde_json::to_string(&LocateRequest { cmd: "locate" })?;
        self.request(&req)
    }

    /// Look up specific BSSIDs (ephemeral, does not write to cache).
    pub fn resolve(&self, bssids: Vec<String>) -> Result<ResolveResponse> {
        let req = serde_json::to_string(&ResolveRequest {
            cmd: "resolve",
            bssids,
        })?;
        self.request(&req)
    }

    /// Get current visible Wi-Fi networks.
    pub fn scan(&self) -> Result<ScanResponse> {
        let req = serde_json::to_string(&SimpleRequest { cmd: "scan" })?;
        self.request(&req)
    }

    /// Get cache and API statistics.
    pub fn stats(&self) -> Result<StatsResponse> {
        let req = serde_json::to_string(&SimpleRequest { cmd: "stats" })?;
        self.request(&req)
    }

    /// Get the daemon's debug snapshot: scan buffer state, per-BSSID
    /// debounce counters, DB classification.
    pub fn debug(&self) -> Result<DebugResponse> {
        let req = serde_json::to_string(&SimpleRequest { cmd: "debug" })?;
        self.request(&req)
    }

    /// Get the daemon's version string and git revision.
    pub fn version(&self) -> Result<VersionResponse> {
        let req = serde_json::to_string(&SimpleRequest { cmd: "version" })?;
        self.request(&req)
    }

    /// Query location history. Pass either `range` ("7d", "24h") or both
    /// `from` and `to` as RFC3339 timestamps; not both.
    pub fn history(
        &self,
        range: Option<String>,
        from: Option<String>,
        to: Option<String>,
    ) -> Result<HistoryResponse> {
        let req = serde_json::to_string(&HistoryRequest {
            cmd: "history",
            range,
            from,
            to,
        })?;
        self.request(&req)
    }
}

// task-0058: trait impls for every response so CLIs can dispatch
// uniformly via `if !resp.is_ok() { fatal(resp.error()) }`.
macro_rules! impl_daemon_response {
    ($t:ty) => {
        impl DaemonResponse for $t {
            fn is_ok(&self) -> bool {
                self.ok
            }
            fn error(&self) -> Option<&str> {
                self.error.as_deref()
            }
        }
    };
}
impl_daemon_response!(LocateResponse);
impl_daemon_response!(ResolveResponse);
impl_daemon_response!(ScanResponse);
impl_daemon_response!(StatsResponse);
impl_daemon_response!(DebugResponse);
impl_daemon_response!(VersionResponse);
impl_daemon_response!(HistoryResponse);
