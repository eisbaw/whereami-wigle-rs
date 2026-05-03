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
#[derive(Deserialize, Debug)]
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
#[derive(Deserialize, Debug)]
pub struct NetworkEntry {
    pub bssid: String,
    pub ssid: Option<String>,
    pub signal_dbm: i32,
    pub channel: Option<i32>,
}

/// Response from the `scan` command.
#[derive(Deserialize, Debug)]
pub struct ScanResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub networks: Vec<NetworkEntry>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Response from the `stats` command.
#[derive(Deserialize, Debug)]
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
}
