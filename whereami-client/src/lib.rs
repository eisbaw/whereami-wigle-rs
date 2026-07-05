//! whereami-client: thin client for the whereamid daemon.
//!
//! Connect to the daemon, send a JSON command, read a JSON response. Each
//! method opens a new connection (one-shot protocol). The default transport is
//! a Unix domain socket, gated by filesystem permissions (ADR-003); TCP is
//! opt-in via `$WHEREAMI_ADDR` for old daemons or custom deployments.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the client connects to reach the daemon (task-0196). The default
/// transport is a Unix domain socket; TCP is opt-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectTarget {
    /// Unix domain socket path (the default, access-controlled transport).
    Unix(PathBuf),
    /// TCP `host:port` (opt-in, unauthenticated).
    Tcp(String),
}

/// Environment override naming an explicit unix socket path (highest priority).
const ENV_SOCKET: &str = "WHEREAMI_SOCKET";
/// Environment override naming a TCP `host:port` (old TCP daemons / custom
/// deployments).
const ENV_ADDR: &str = "WHEREAMI_ADDR";
/// Default socket filename under `$XDG_RUNTIME_DIR` and `/run/whereami`.
const SOCKET_FILENAME: &str = "whereamid.sock";
/// System-wide default socket path (used when `$XDG_RUNTIME_DIR` is unset).
const RUN_SOCKET: &str = "/run/whereami/whereamid.sock";
/// Default timeout for each blocking socket read or write.
///
/// Forty seconds covers the nominal 35-second envelope for one default
/// provider chain (up to 15 seconds each for Apple and WiGLE, then the daemon's
/// five-second response-write budget) plus five seconds of scheduling and
/// database margin. It is not an end-to-end upper bound: multi-BSSID requests,
/// custom provider timeouts, and large history queries may legitimately need
/// longer. Use [`WhereAmIClient::with_io_timeout`] to raise it or
/// [`WhereAmIClient::without_io_timeout`] to disable it explicitly.
pub const DEFAULT_SOCKET_IO_TIMEOUT: Duration = Duration::from_secs(40);

/// Client for the whereamid daemon.
///
/// New clients use [`DEFAULT_SOCKET_IO_TIMEOUT`] for every blocking socket
/// read and write. The setting applies uniformly to typed and raw commands over
/// both TCP and Unix sockets.
pub struct WhereAmIClient {
    target: ConnectTarget,
    io_timeout: Option<Duration>,
}

/// Resolve the default connect target from the environment (task-0196). Pure:
/// env values and the existence probe are injected, so it never reads the
/// host's real `$XDG_RUNTIME_DIR` in tests.
///
/// Order: `$WHEREAMI_SOCKET` (unix path, verbatim) → `$WHEREAMI_ADDR` (TCP) →
/// first EXISTING of `$XDG_RUNTIME_DIR/whereamid.sock`, `/run/whereami/whereamid.sock`
/// → error naming the probed paths and the overrides.
///
/// There is deliberately NO fallback to `127.0.0.1:4747`: after the daemon
/// vacates that port, any local user could squat it and impersonate the daemon
/// (an integrity downgrade). See ADR-003.
fn resolve_target(
    whereami_socket: Option<std::ffi::OsString>,
    whereami_addr: Option<std::ffi::OsString>,
    xdg_runtime_dir: Option<std::ffi::OsString>,
    exists: impl Fn(&Path) -> bool,
) -> Result<ConnectTarget> {
    if let Some(s) = whereami_socket {
        if !s.is_empty() {
            return Ok(ConnectTarget::Unix(PathBuf::from(s)));
        }
    }
    if let Some(a) = whereami_addr {
        if !a.is_empty() {
            return Ok(ConnectTarget::Tcp(a.to_string_lossy().into_owned()));
        }
    }
    let mut probed: Vec<PathBuf> = Vec::new();
    if let Some(dir) = xdg_runtime_dir {
        if !dir.is_empty() {
            let p = Path::new(&dir).join(SOCKET_FILENAME);
            if exists(&p) {
                return Ok(ConnectTarget::Unix(p));
            }
            probed.push(p);
        }
    }
    let run = PathBuf::from(RUN_SOCKET);
    if exists(&run) {
        return Ok(ConnectTarget::Unix(run));
    }
    probed.push(run);

    let probed_list = probed
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    bail!(
        "no whereamid control socket found (probed: {probed_list}). Start whereamid, or set \
         {ENV_SOCKET}=<socket path> or {ENV_ADDR}=<host:port> to point at it."
    )
}

/// Transport abstraction so the write→shutdown→read exchange is written once
/// for both TCP and unix streams (their `shutdown` is an inherent method, not a
/// shared trait). task-0196.
trait Transport: Read + Write {
    fn shutdown_write(&self) -> std::io::Result<()>;
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
}
impl Transport for TcpStream {
    fn shutdown_write(&self) -> std::io::Result<()> {
        self.shutdown(Shutdown::Write)
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        TcpStream::set_write_timeout(self, timeout)
    }
}
impl Transport for UnixStream {
    fn shutdown_write(&self) -> std::io::Result<()> {
        self.shutdown(Shutdown::Write)
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        UnixStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        UnixStream::set_write_timeout(self, timeout)
    }
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

const WIPE_CONFIRMATION_TOKEN: &str = "delete-all-location-data";

/// Explicit confirmation marker required by the typed destructive wipe API.
/// This prevents accidental call sites; Unix-socket permissions, not this
/// value, provide server-side authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WipeConfirmation {
    Confirmed,
}

#[derive(Serialize)]
struct WipeRequest {
    cmd: &'static str,
    confirm: &'static str,
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

/// Provenance of a `resolve` result (task-0062).
///
/// Disambiguated from the daemon's data-`Source` enum (apple/wigle/...)
/// by the rename: the wire field is still `source` for backward compat
/// but the typed Rust field on this struct is `provenance`. An unknown
/// value from a future daemon maps to `Provenance::Unknown` so older
/// clients keep parsing.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Fetched live during the request.
    Api,
    /// Served from the daemon's local cache.
    Cache,
    /// Authoritatively absent: a provider returned NotFound this call, or the
    /// daemon's not_found table recorded absence within its TTL.
    NotFound,
    /// task-0194: the daemon could not consult a provider for this BSSID this
    /// call (deferred to an in-flight resolution, provider skipped for missing
    /// creds / exhausted or corrupt quota, or a transient failure), so absence
    /// is NOT established. Retry later rather than caching it as absent.
    Unresolved,
    /// A value the daemon emitted that this client version does not
    /// recognise. Forward-compat escape hatch (`#[serde(other)]`).
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for Provenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Provenance::Api => "api",
            Provenance::Cache => "cache",
            Provenance::NotFound => "not_found",
            Provenance::Unresolved => "unresolved",
            Provenance::Unknown => "unknown",
        })
    }
}

/// A single result from the `resolve` command.
#[derive(Serialize, Deserialize, Debug)]
pub struct ResolveResultEntry {
    pub bssid: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub ssid: Option<String>,
    /// Wire field name is `source` for backward compat. task-0062 renamed
    /// the Rust field to `provenance` to disambiguate from the data-
    /// source `Source` enum (apple/wigle/...).
    #[serde(rename = "source")]
    pub provenance: Provenance,
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

/// DB classification of a BSSID seen by the debouncer (task-0062).
///
/// Wire format mirrors the daemon's snake_case strings. An unknown
/// value from a future daemon maps to `DbStatus::Unknown` so older
/// clients keep parsing.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DbStatus {
    /// In the daemon's `aps` cache.
    Cached,
    /// Queued for resolution.
    Pending,
    /// In the daemon's `not_found` table (definitively absent within TTL).
    NotFound,
    /// First-seen this scan.
    New,
    /// A value the daemon emitted that this client version does not
    /// recognise. Forward-compat escape hatch (`#[serde(other)]`).
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for DbStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DbStatus::Cached => "cached",
            DbStatus::Pending => "pending",
            DbStatus::NotFound => "not_found",
            DbStatus::New => "new",
            DbStatus::Unknown => "unknown",
        })
    }
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
    pub db_status: DbStatus,
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
    /// Executed WiGLE reservations denied since daemon start because the
    /// same-day persisted API counter was malformed. Distinct from normal
    /// quota exhaustion and defaults to 0 for older daemons.
    #[serde(default)]
    pub api_counter_corruption_denials: u64,
    /// In-flight provider lookups currently coalescing concurrent requests.
    /// task-0074. Defaults to 0 for older daemons that don't emit it.
    #[serde(default)]
    pub inflight: usize,
    /// Cumulative count of best-effort DB writes that failed since daemon
    /// start. task-0076. Defaults to 0 for older daemons.
    #[serde(default)]
    pub db_write_failures: u64,
    /// Number of tasks force-aborted at the end of the last clean daemon
    /// shutdown. This is one persisted shutdown snapshot, neither a current
    /// count nor a cumulative total. The daemon reloads it on startup; a crash
    /// or SIGKILL does not update it. Defaults to 0 for older daemons that do
    /// not emit the field.
    #[serde(default)]
    pub orphan_tasks_aborted: u64,
    #[serde(default)]
    pub error: Option<String>,
}

/// Response from a confirmed privacy wipe.
#[derive(Serialize, Deserialize, Debug)]
pub struct WipeResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub aps_deleted: usize,
    #[serde(default)]
    pub pending_deleted: usize,
    #[serde(default)]
    pub not_found_deleted: usize,
    #[serde(default)]
    pub fixes_deleted: usize,
    #[serde(default)]
    pub last_fix_deleted: usize,
    #[serde(default)]
    pub error: Option<String>,
}

impl WhereAmIClient {
    /// Create a client for an explicit connect target, using
    /// [`DEFAULT_SOCKET_IO_TIMEOUT`].
    pub fn new(target: ConnectTarget) -> Self {
        Self {
            target,
            io_timeout: Some(DEFAULT_SOCKET_IO_TIMEOUT),
        }
    }

    /// Convenience: a TCP client for `host:port` (opt-in, unauthenticated).
    pub fn tcp(addr: &str) -> Self {
        Self::new(ConnectTarget::Tcp(addr.to_string()))
    }

    /// Convenience: a unix-socket client for `path`.
    pub fn unix(path: impl Into<PathBuf>) -> Self {
        Self::new(ConnectTarget::Unix(path.into()))
    }

    /// Resolve the default target from the environment using the documented
    /// socket-resolution order.
    /// Errors when no daemon socket can be found and no override is set; the
    /// caller decides how to surface that (the CLI routes it through the same
    /// `--json`/human path as a transport error, task-0183).
    pub fn from_env() -> Result<Self> {
        let target = resolve_target(
            std::env::var_os(ENV_SOCKET),
            std::env::var_os(ENV_ADDR),
            std::env::var_os("XDG_RUNTIME_DIR"),
            |p| p.exists(),
        )?;
        Ok(Self::new(target))
    }

    /// The target this client connects to.
    pub fn target(&self) -> &ConnectTarget {
        &self.target
    }

    /// Return the configured timeout for each blocking socket read and write.
    /// `None` means socket I/O timeouts are disabled.
    pub fn io_timeout(&self) -> Option<Duration> {
        self.io_timeout
    }

    /// Set a positive timeout for each blocking socket read and write.
    ///
    /// This is not a total request deadline. Raise it for multi-BSSID requests,
    /// large history queries, or daemons with longer provider timeouts. To
    /// disable socket I/O timeouts explicitly, call
    /// [`WhereAmIClient::without_io_timeout`] instead of passing zero.
    pub fn with_io_timeout(mut self, timeout: Duration) -> Result<Self> {
        if timeout.is_zero() {
            bail!("socket I/O timeout must be positive; use without_io_timeout() to disable it");
        }
        self.io_timeout = Some(timeout);
        Ok(self)
    }

    /// Disable socket read and write timeouts.
    ///
    /// A daemon that stalls or never responds can then block the calling thread
    /// indefinitely. This is intended for explicitly supervised long-running
    /// batch or history operations.
    pub fn without_io_timeout(mut self) -> Self {
        self.io_timeout = None;
        self
    }

    /// Send a raw JSON command and return the raw response string. Dispatches
    /// on the transport; the exchange itself is transport-agnostic.
    pub fn raw_command(&self, json: &str) -> Result<String> {
        match &self.target {
            ConnectTarget::Tcp(addr) => {
                let stream = TcpStream::connect(addr)
                    .with_context(|| format!("connecting to whereamid at {addr}"))?;
                self.exchange(stream, json)
            }
            ConnectTarget::Unix(path) => {
                let stream = UnixStream::connect(path)
                    .with_context(|| format!("connecting to whereamid at {}", path.display()))?;
                self.exchange(stream, json)
            }
        }
    }

    /// One-shot request/response over any transport: apply the common socket
    /// I/O budget, write the request + newline, half-close the write side so
    /// the daemon sees EOF, then read exactly one response line.
    fn exchange<S: Transport>(&self, mut stream: S, json: &str) -> Result<String> {
        stream
            .set_read_timeout(self.io_timeout)
            .context("configuring daemon socket read timeout")?;
        stream
            .set_write_timeout(self.io_timeout)
            .context("configuring daemon socket write timeout")?;
        stream
            .write_all(json.as_bytes())
            .context("sending request")?;
        stream.write_all(b"\n").context("sending newline")?;
        stream.flush().context("flushing")?;
        stream.shutdown_write().context("shutdown write")?;
        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        if let Err(error) = reader.read_line(&mut response_line) {
            if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) {
                if let Some(timeout) = self.io_timeout {
                    return Err(anyhow::Error::new(error).context(format!(
                        "timed out after {timeout:?} waiting for a response from whereamid"
                    )));
                }
            }
            return Err(error).context("reading response");
        }
        Ok(response_line.trim().to_string())
    }

    /// Parse a daemon response line into the typed response `T`.
    ///
    /// Every typed accessor fetches its raw line via the matching
    /// `*_raw()` method (which builds the request struct and calls
    /// `raw_command`) and parses it here. That keeps the typed path and
    /// the `--json` passthrough on a single source of truth for both the
    /// request bytes and the transport (task-0183): they can no longer
    /// drift because there is exactly one place each request is built.
    fn parse_response_line<T: for<'de> Deserialize<'de>>(line: &str) -> Result<T> {
        if line.is_empty() {
            bail!("empty response from daemon");
        }
        serde_json::from_str(line).context("parsing response JSON")
    }

    /// Ask "where am I?" based on current stable APs.
    pub fn locate(&self) -> Result<LocateResponse> {
        Self::parse_response_line(&self.locate_raw()?)
    }

    /// Raw `locate`: daemon JSON response body after transport framing and
    /// surrounding-whitespace removal; feeds `--json`.
    pub fn locate_raw(&self) -> Result<String> {
        self.raw_command(&serde_json::to_string(&LocateRequest { cmd: "locate" })?)
    }

    /// Look up specific BSSIDs (ephemeral, does not write to cache).
    pub fn resolve(&self, bssids: Vec<String>) -> Result<ResolveResponse> {
        Self::parse_response_line(&self.resolve_raw(bssids)?)
    }

    /// Raw `resolve`: daemon JSON response body after transport framing and
    /// surrounding-whitespace removal; feeds `--json`.
    pub fn resolve_raw(&self, bssids: Vec<String>) -> Result<String> {
        self.raw_command(&serde_json::to_string(&ResolveRequest {
            cmd: "resolve",
            bssids,
        })?)
    }

    /// Get current visible Wi-Fi networks.
    pub fn scan(&self) -> Result<ScanResponse> {
        Self::parse_response_line(&self.scan_raw()?)
    }

    /// Raw `scan`: daemon JSON response body after transport framing and
    /// surrounding-whitespace removal; feeds `--json`.
    pub fn scan_raw(&self) -> Result<String> {
        self.raw_command(&serde_json::to_string(&SimpleRequest { cmd: "scan" })?)
    }

    /// Get cache and API statistics.
    pub fn stats(&self) -> Result<StatsResponse> {
        Self::parse_response_line(&self.stats_raw()?)
    }

    /// Raw `stats`: daemon JSON response body after transport framing and
    /// surrounding-whitespace removal; feeds `--json`.
    pub fn stats_raw(&self) -> Result<String> {
        self.raw_command(&serde_json::to_string(&SimpleRequest { cmd: "stats" })?)
    }

    /// Get the daemon's debug snapshot: scan buffer state, per-BSSID
    /// debounce counters, DB classification.
    pub fn debug(&self) -> Result<DebugResponse> {
        Self::parse_response_line(&self.debug_raw()?)
    }

    /// Raw `debug`: daemon JSON response body after transport framing and
    /// surrounding-whitespace removal; feeds `--json`.
    pub fn debug_raw(&self) -> Result<String> {
        self.raw_command(&serde_json::to_string(&SimpleRequest { cmd: "debug" })?)
    }

    /// Get the daemon's version string and git revision.
    pub fn version(&self) -> Result<VersionResponse> {
        Self::parse_response_line(&self.version_raw()?)
    }

    /// Raw `version`: daemon JSON response body after transport framing and
    /// surrounding-whitespace removal; feeds `--json`.
    pub fn version_raw(&self) -> Result<String> {
        self.raw_command(&serde_json::to_string(&SimpleRequest { cmd: "version" })?)
    }

    /// Query location history. Pass either `range` ("7d", "24h") or both
    /// `from` and `to` as RFC3339 timestamps; not both.
    pub fn history(
        &self,
        range: Option<String>,
        from: Option<String>,
        to: Option<String>,
    ) -> Result<HistoryResponse> {
        Self::parse_response_line(&self.history_raw(range, from, to)?)
    }

    /// Raw `history`: daemon JSON response body after transport framing and
    /// surrounding-whitespace removal; feeds `--json`.
    pub fn history_raw(
        &self,
        range: Option<String>,
        from: Option<String>,
        to: Option<String>,
    ) -> Result<String> {
        self.raw_command(&serde_json::to_string(&HistoryRequest {
            cmd: "history",
            range,
            from,
            to,
        })?)
    }

    /// Delete the daemon's persisted and managed in-memory location/AP state.
    /// The explicit enum makes destructive intent visible at every call site.
    pub fn wipe(&self, confirmation: WipeConfirmation) -> Result<WipeResponse> {
        Self::parse_response_line(&self.wipe_raw(confirmation)?)
    }

    /// Raw confirmed `wipe`; feeds the CLI's byte-preserving `--json` mode.
    pub fn wipe_raw(&self, confirmation: WipeConfirmation) -> Result<String> {
        let confirm = match confirmation {
            WipeConfirmation::Confirmed => WIPE_CONFIRMATION_TOKEN,
        };
        self.raw_command(&serde_json::to_string(&WipeRequest {
            cmd: "wipe",
            confirm,
        })?)
    }
}

#[cfg(test)]
mod socket_timeout_tests {
    use super::*;
    use std::io;
    use std::net::{SocketAddr, TcpListener};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread::{self, JoinHandle};
    use std::time::Instant;

    /// A real TCP peer that consumes the half-closed request but deliberately
    /// keeps its response side open. The release channel, rather than a sleep,
    /// controls the peer lifetime so timeout assertions do not leak a thread.
    struct SilentTcpServer {
        address: SocketAddr,
        request_rx: Receiver<String>,
        release_tx: Option<Sender<()>>,
        thread: Option<JoinHandle<io::Result<()>>>,
    }

    impl SilentTcpServer {
        fn start() -> io::Result<Self> {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let address = listener.local_addr()?;
            let (request_tx, request_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let thread = thread::spawn(move || {
                let (mut stream, _) = listener.accept()?;
                // Keep fixture cleanup finite even if request framing or the
                // client's write-side half-close regresses.
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                let mut request = String::new();
                stream.read_to_string(&mut request)?;
                request_tx.send(request).map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!("reporting observed request: {error}"),
                    )
                })?;
                release_rx
                    .recv_timeout(Duration::from_secs(10))
                    .map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!("waiting for test release: {error}"),
                        )
                    })?;
                Ok(())
            });
            Ok(Self {
                address,
                request_rx,
                release_tx: Some(release_tx),
                thread: Some(thread),
            })
        }

        fn address(&self) -> SocketAddr {
            self.address
        }

        fn release_and_join(&mut self) -> io::Result<()> {
            let Some(thread) = self.thread.take() else {
                return Ok(());
            };

            if let Some(release_tx) = self.release_tx.take() {
                let _ = release_tx.send(());
            }
            // If the client failed before connecting, this cleanup connection
            // unblocks accept/read so even a failing assertion can join.
            if let Ok(stream) = TcpStream::connect(self.address) {
                let _ = stream.shutdown(Shutdown::Write);
            }
            thread
                .join()
                .map_err(|_| io::Error::other("silent TCP server thread panicked"))?
        }

        fn finish(mut self) -> io::Result<String> {
            let request = self
                .request_rx
                .recv_timeout(Duration::from_secs(1))
                .map_err(|error| io::Error::other(format!("receiving observed request: {error}")));
            let server_result = self.release_and_join();
            server_result?;
            request
        }
    }

    impl Drop for SilentTcpServer {
        fn drop(&mut self) {
            let _ = self.release_and_join();
        }
    }

    #[test]
    fn unix_transport_forwards_socket_timeout_configuration() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let custom_timeout = Duration::from_secs(7);

        Transport::set_read_timeout(&stream, Some(custom_timeout)).unwrap();
        Transport::set_write_timeout(&stream, Some(custom_timeout)).unwrap();
        assert_eq!(stream.read_timeout().unwrap(), Some(custom_timeout));
        assert_eq!(stream.write_timeout().unwrap(), Some(custom_timeout));

        Transport::set_read_timeout(&stream, None).unwrap();
        Transport::set_write_timeout(&stream, None).unwrap();
        assert_eq!(stream.read_timeout().unwrap(), None);
        assert_eq!(stream.write_timeout().unwrap(), None);
    }

    #[test]
    fn client_timeout_builders_expose_default_custom_and_disabled_states() {
        let default_client = WhereAmIClient::tcp("127.0.0.1:1");
        assert_eq!(DEFAULT_SOCKET_IO_TIMEOUT, Duration::from_secs(40));
        assert_eq!(default_client.io_timeout(), Some(Duration::from_secs(40)));

        let custom_client = default_client
            .with_io_timeout(Duration::from_secs(90))
            .unwrap();
        assert_eq!(custom_client.io_timeout(), Some(Duration::from_secs(90)));

        let disabled_client = custom_client.without_io_timeout();
        assert_eq!(disabled_client.io_timeout(), None);

        let error = WhereAmIClient::tcp("127.0.0.1:1")
            .with_io_timeout(Duration::ZERO)
            .err()
            .expect("zero must require the explicit disable builder");
        assert_eq!(
            error.to_string(),
            "socket I/O timeout must be positive; use without_io_timeout() to disable it"
        );
    }

    #[test]
    fn typed_locate_times_out_when_tcp_daemon_never_responds() {
        const TEST_IO_TIMEOUT: Duration = Duration::from_millis(250);
        let server = SilentTcpServer::start().unwrap();
        let address = server.address().to_string();
        let (outcome_tx, outcome_rx) = mpsc::channel();
        let client_thread = thread::spawn(move || {
            let client = WhereAmIClient::tcp(&address)
                .with_io_timeout(TEST_IO_TIMEOUT)
                .unwrap();
            assert_eq!(client.io_timeout(), Some(TEST_IO_TIMEOUT));
            let started = Instant::now();
            let result = client.locate();
            let _ = outcome_tx.send((result, started.elapsed()));
        });

        // This watchdog is deliberately independent of the production socket
        // timeout. If timeout configuration regresses, releasing the silent
        // peer closes its response side and lets us join the client thread
        // before failing instead of hanging the test suite forever.
        let outcome = outcome_rx.recv_timeout(Duration::from_secs(5));
        let request_result = server.finish();
        let client_thread_result = client_thread.join();
        assert!(
            client_thread_result.is_ok(),
            "timeout-test client thread panicked"
        );
        let (result, elapsed) = outcome.unwrap_or_else(|error| {
            panic!("client did not return within the independent 5s watchdog: {error}")
        });
        let request = request_result.unwrap();

        assert_eq!(request, "{\"cmd\":\"locate\"}\n");
        assert!(
            elapsed >= Duration::from_millis(200),
            "client returned before its configured 250ms timeout: {elapsed:?}"
        );
        assert!(
            elapsed <= Duration::from_secs(2),
            "client exceeded the bounded timeout allowance: {elapsed:?}"
        );

        let error = result.expect_err("a silent daemon must time out");
        assert_eq!(
            error.to_string(),
            "timed out after 250ms waiting for a response from whereamid"
        );
        let io_error = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<io::Error>())
            .expect("timeout must retain the original io::Error source");
        assert!(
            matches!(
                io_error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ),
            "unexpected timeout error kind: {:?}",
            io_error.kind()
        );
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
impl_daemon_response!(WipeResponse);

#[cfg(test)]
mod wipe_wire_tests {
    use super::*;

    #[test]
    fn confirmed_wipe_request_is_exact() {
        let request = WipeRequest {
            cmd: "wipe",
            confirm: match WipeConfirmation::Confirmed {
                WipeConfirmation::Confirmed => WIPE_CONFIRMATION_TOKEN,
            },
        };
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"cmd":"wipe","confirm":"delete-all-location-data"}"#
        );
    }

    #[test]
    fn wipe_success_and_error_shapes_parse_typed() {
        let success: WipeResponse = WhereAmIClient::parse_response_line(
            r#"{"ok":true,"v":1,"aps_deleted":5,"pending_deleted":4,"not_found_deleted":3,"fixes_deleted":2,"last_fix_deleted":1}"#,
        )
        .unwrap();
        assert!(success.ok);
        assert_eq!(success.aps_deleted, 5);
        assert_eq!(success.last_fix_deleted, 1);

        let error: WipeResponse =
            WhereAmIClient::parse_response_line(r#"{"ok":false,"v":1,"error":"wipe refused"}"#)
                .unwrap();
        assert!(!error.ok);
        assert_eq!(error.error.as_deref(), Some("wipe refused"));
        assert_eq!(error.fixes_deleted, 0);
    }
}

#[cfg(test)]
mod stats_wire_tests {
    //! TASK-0243: the typed stats response stays aligned with the canonical
    //! daemon success shape while remaining compatible with older daemons.
    use super::*;

    #[test]
    fn current_daemon_stats_shape_deserializes_every_typed_field() {
        let json = r#"{"ok":true,"v":1,"cached_aps":41,"pending_aps":2,"not_found_aps":3,"db_size_bytes":49152,"api_calls_today":4,"api_counter_corruption_denials":8,"inflight":5,"db_write_failures":6,"orphan_tasks_aborted":7}"#;
        let stats: StatsResponse = WhereAmIClient::parse_response_line(json).unwrap();

        assert!(stats.ok);
        assert_eq!(stats.v, 1);
        assert_eq!(stats.cached_aps, 41);
        assert_eq!(stats.pending_aps, 2);
        assert_eq!(stats.not_found_aps, 3);
        assert_eq!(stats.db_size_bytes, 49_152);
        assert_eq!(stats.api_calls_today, 4);
        assert_eq!(stats.api_counter_corruption_denials, 8);
        assert_eq!(stats.inflight, 5);
        assert_eq!(stats.db_write_failures, 6);
        assert_eq!(stats.orphan_tasks_aborted, 7);
        assert!(stats.error.is_none());
    }

    #[test]
    fn legacy_stats_shape_defaults_missing_last_shutdown_aborts_to_zero() {
        let json = r#"{"ok":true,"v":1,"cached_aps":101,"pending_aps":102,"not_found_aps":103,"db_size_bytes":104,"api_calls_today":105,"inflight":106,"db_write_failures":107}"#;
        let stats: StatsResponse = WhereAmIClient::parse_response_line(json).unwrap();

        assert!(stats.ok);
        assert_eq!(stats.v, 1);
        assert_eq!(stats.cached_aps, 101);
        assert_eq!(stats.pending_aps, 102);
        assert_eq!(stats.not_found_aps, 103);
        assert_eq!(stats.db_size_bytes, 104);
        assert_eq!(stats.api_calls_today, 105);
        assert_eq!(stats.api_counter_corruption_denials, 0);
        assert_eq!(stats.inflight, 106);
        assert_eq!(stats.db_write_failures, 107);
        assert_eq!(stats.orphan_tasks_aborted, 0);
        assert!(stats.error.is_none());
    }
}

#[cfg(test)]
mod wire_enum_tests {
    //! task-0062: round-trip the typed wire enums against the strings the
    //! daemon emits, including the Unknown forward-compat fallback.
    use super::*;

    #[test]
    fn db_status_strings_match_daemon_vocabulary() {
        for (variant, expected) in [
            (DbStatus::Cached, "\"cached\""),
            (DbStatus::Pending, "\"pending\""),
            (DbStatus::NotFound, "\"not_found\""),
            (DbStatus::New, "\"new\""),
        ] {
            assert_eq!(serde_json::to_string(&variant).unwrap(), expected);
            let back: DbStatus = serde_json::from_str(expected).unwrap();
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn db_status_unknown_string_maps_to_unknown_variant() {
        // Forward-compat: a future daemon emitting a new variant must not
        // break clients linked against this version.
        let v: DbStatus = serde_json::from_str("\"future_variant\"").unwrap();
        assert_eq!(v, DbStatus::Unknown);
    }

    #[test]
    fn provenance_strings_match_daemon_vocabulary() {
        for (variant, expected) in [
            (Provenance::Api, "\"api\""),
            (Provenance::Cache, "\"cache\""),
            (Provenance::NotFound, "\"not_found\""),
            // task-0194: daemon emits `unresolved` for deferred/skipped/
            // transient BSSIDs (absence not established).
            (Provenance::Unresolved, "\"unresolved\""),
        ] {
            assert_eq!(serde_json::to_string(&variant).unwrap(), expected);
            let back: Provenance = serde_json::from_str(expected).unwrap();
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn provenance_unknown_string_maps_to_unknown_variant() {
        let v: Provenance = serde_json::from_str("\"future_variant\"").unwrap();
        assert_eq!(v, Provenance::Unknown);
    }

    #[test]
    fn resolve_result_entry_reads_legacy_source_field() {
        // The wire still uses `source`; the typed Rust field is `provenance`.
        // A client must parse responses produced by the current daemon.
        let json =
            r#"{"bssid":"aa:bb:cc:dd:ee:ff","lat":1.0,"lon":2.0,"ssid":"x","source":"cache"}"#;
        let r: ResolveResultEntry = serde_json::from_str(json).unwrap();
        assert_eq!(r.provenance, Provenance::Cache);
    }

    #[test]
    fn resolve_result_entry_emits_source_field_name() {
        let r = ResolveResultEntry {
            bssid: "aa:bb:cc:dd:ee:ff".to_string(),
            lat: Some(1.0),
            lon: Some(2.0),
            ssid: None,
            provenance: Provenance::Api,
        };
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v.get("source").and_then(|x| x.as_str()), Some("api"));
        assert!(v.get("provenance").is_none());
    }

    #[test]
    fn debug_bssid_round_trip_with_typed_db_status() {
        let json = r#"{"bssid":"aa:bb:cc:dd:ee:ff","signal_dbm":-50,"seen":3,"needed":3,"is_stable":true,"db_status":"cached"}"#;
        let d: DebugBssid = serde_json::from_str(json).unwrap();
        assert_eq!(d.db_status, DbStatus::Cached);
        // Round-trip back to JSON and re-parse.
        let s = serde_json::to_string(&d).unwrap();
        let d2: DebugBssid = serde_json::from_str(&s).unwrap();
        assert_eq!(d2.db_status, DbStatus::Cached);
    }
}

#[cfg(test)]
mod target_resolution_tests {
    //! task-0196: default connect-target resolution order, exercised purely —
    //! env values and the existence probe are injected, so the tests never
    //! touch the host's real XDG_RUNTIME_DIR, filesystem defaults, or a live
    //! daemon.
    use super::*;
    use std::ffi::OsString;

    fn os(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    #[test]
    fn socket_env_wins_over_everything() {
        let t = resolve_target(
            os("/run/user/1000/custom.sock"),
            os("10.0.0.5:4747"),
            os("/run/user/1000"),
            |_| true,
        )
        .unwrap();
        assert_eq!(
            t,
            ConnectTarget::Unix(PathBuf::from("/run/user/1000/custom.sock"))
        );
    }

    #[test]
    fn socket_env_used_verbatim_without_existence_check() {
        // WHEREAMI_SOCKET is used even when the path does not exist (exists
        // closure returns false); the connect error surfaces naturally later.
        let t = resolve_target(os("/nonexistent/x.sock"), None, None, |_| false).unwrap();
        assert_eq!(t, ConnectTarget::Unix(PathBuf::from("/nonexistent/x.sock")));
    }

    #[test]
    fn addr_env_selects_tcp_when_no_socket_env() {
        let t = resolve_target(None, os("10.0.0.5:4747"), os("/run/user/1000"), |_| true).unwrap();
        assert_eq!(t, ConnectTarget::Tcp("10.0.0.5:4747".to_string()));
    }

    #[test]
    fn xdg_socket_used_when_it_exists() {
        let t = resolve_target(None, None, os("/run/user/1000"), |p| {
            p == Path::new("/run/user/1000/whereamid.sock")
        })
        .unwrap();
        assert_eq!(
            t,
            ConnectTarget::Unix(PathBuf::from("/run/user/1000/whereamid.sock"))
        );
    }

    #[test]
    fn falls_back_to_run_socket_when_xdg_socket_absent() {
        let t = resolve_target(None, None, os("/run/user/1000"), |p| {
            p == Path::new(RUN_SOCKET)
        })
        .unwrap();
        assert_eq!(t, ConnectTarget::Unix(PathBuf::from(RUN_SOCKET)));
    }

    #[test]
    fn errors_listing_both_probed_paths_and_overrides() {
        let err = resolve_target(None, None, os("/run/user/1000"), |_| false).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("/run/user/1000/whereamid.sock"),
            "must list the probed XDG path: {msg}"
        );
        assert!(
            msg.contains(RUN_SOCKET),
            "must list the probed /run path: {msg}"
        );
        assert!(
            msg.contains(ENV_SOCKET) && msg.contains(ENV_ADDR),
            "must mention both env overrides: {msg}"
        );
    }

    #[test]
    fn never_falls_back_to_tcp_4747() {
        // Integrity-critical: no implicit 127.0.0.1:4747 (port-squat
        // impersonation, ADR-003). With nothing set the result is an error, and
        // it must not suggest the old TCP port.
        let err = resolve_target(None, None, None, |_| false).unwrap_err();
        assert!(
            !format!("{err}").contains("4747"),
            "resolution must never mention the old default TCP port"
        );
    }

    #[test]
    fn empty_env_values_are_ignored() {
        // Empty WHEREAMI_SOCKET / WHEREAMI_ADDR must not select an empty target.
        let t = resolve_target(os(""), os(""), os("/run/user/1000"), |p| {
            p == Path::new("/run/user/1000/whereamid.sock")
        })
        .unwrap();
        assert_eq!(
            t,
            ConnectTarget::Unix(PathBuf::from("/run/user/1000/whereamid.sock"))
        );
    }
}
