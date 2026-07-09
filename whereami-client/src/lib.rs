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

impl ConnectTarget {
    /// Human-readable endpoint description for transport and protocol errors.
    fn description(&self) -> String {
        match self {
            Self::Unix(path) => format!("unix socket {}", path.display()),
            Self::Tcp(addr) => format!("TCP endpoint {addr}"),
        }
    }
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

/// A typed daemon-side error (task-0133).
///
/// The daemon reports every failed command as the dedicated
/// `{"ok":false,"v":_,"error":_}` envelope ([`whereami_protocol::ErrorResponse`]),
/// never as an `error` field on a success struct. The client dispatches on the
/// `ok` bool in [`WhereAmIClient::parse_response_line`]: an `ok:false` line is
/// decoded as an error and surfaced as this type rather than as a typed
/// response with defaulted fields — removing the previous fragile
/// coincidence-matching (a success struct that also carried `error`).
///
/// The typed accessors keep returning [`anyhow::Result`] so transport and
/// protocol errors retain full fidelity; a daemon error is wrapped as
/// `Err(DaemonError { .. }.into())`. Callers can recover the typed error via
/// `err.downcast_ref::<DaemonError>()`. Its [`Display`](std::fmt::Display) is
/// exactly the daemon's error string (no prefix), so existing `{:#}`/context
/// rendering of the daemon message is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonError {
    /// Daemon protocol version from the error envelope.
    pub v: u32,
    /// The daemon's error message, preserved verbatim.
    pub error: String,
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error)
    }
}

impl std::error::Error for DaemonError {}

/// Highest daemon protocol version understood by this typed client. A
/// response advertising a higher `v` is rejected by the version gate. Also
/// reused by the binary crate's test fixtures as the value a coherent daemon
/// would emit at the client's max supported version. Breaking wire-format
/// changes bump this in lockstep with the daemon's own emit constant
/// (`whereamid::WIRE_VERSION`; unified under task-0114).
pub const SUPPORTED_PROTOCOL_VERSION: u32 = 1;

/// Required prefix shared by every daemon response: the protocol version and
/// the `ok` flag. Decoded before the command-specific body so (a) a newer
/// protocol cannot be mistaken for a valid response whose defaultable fields
/// happen to be absent, and (b) the client can dispatch on `ok` to decode an
/// error envelope as [`ErrorResponse`] rather than coincidence-matching it
/// into a typed response (task-0133). `ok` is `Option` so a malformed line
/// that omits it still surfaces the version-gate/decode error rather than a
/// deserialize failure on this probe.
#[derive(Deserialize)]
struct ResponsePrefix {
    v: u32,
    #[serde(default)]
    ok: Option<bool>,
}

/// Private typed-response contract used by the common version gate.
trait ProtocolResponse {
    fn protocol_version(&self) -> u32;
}

// --- Request types ---
//
// The outbound request wire shapes are defined once in `whereami-protocol`
// (task-0114); each `*_raw()` method below serializes exactly one of these,
// preserving the task-0183 single-source request contract.
use whereami_protocol::{
    HistoryFixesRequest, HistoryRequest, LocateRequest, ResolveRequest, SimpleRequest,
};

const WIPE_CONFIRMATION_TOKEN: &str = "delete-all-location-data";

/// Explicit confirmation marker required by the typed destructive wipe API.
/// This prevents accidental call sites; Unix-socket permissions, not this
/// value, provide server-side authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WipeConfirmation {
    Confirmed,
}

/// The `wipe` request stays client-side: it is built only here, from a
/// [`WipeConfirmation`], and carries the confirmation token.
#[derive(Serialize)]
struct WipeRequest {
    cmd: &'static str,
    confirm: &'static str,
}

// --- Response types ---
//
// Every response, the shared enums, and `HistoryFix`/`HistoryRange`/
// `HistorySegment`/`NetworkEntry`/`ResolveResultEntry` are re-exported from
// `whereami-protocol` (task-0114), the single source of truth the daemon also
// links. This crate adds only client behavior (the `ProtocolResponse` trait
// impls below, the `ok`-dispatching `DaemonError` surface, and the raw-history
// presence validator).
pub use whereami_protocol::{
    Channel, Confidence, DbStatus, DebugBssid, DebugResponse, ErrorResponse, HistoryFix,
    HistoryFixesResponse, HistoryRange, HistoryResponse, HistorySegment, LocateResponse,
    NetworkEntry, Provenance, ResolveResponse, ResolveResultEntry, ScanResponse, StatsResponse,
    VersionResponse, WipeResponse, MAX_RESOLVE_BSSIDS,
};

/// Presence-checking wire view. Public response fields remain defaultable so
/// normal `ok:false` envelopes parse like every other client API; successful
/// raw history must explicitly contain both shape-defining containers. The
/// marker visitors walk their contents through `IgnoredAny`: JSON raw mode
/// validates object/array type and non-null presence without allocating a
/// second fix vector, while the typed API subsequently performs full field
/// deserialization into `HistoryFixesResponse`.
#[derive(Deserialize)]
struct HistoryFixesShape {
    ok: bool,
    #[serde(default)]
    range: Option<JsonObjectPresence>,
    #[serde(default)]
    fixes: Option<JsonArrayPresence>,
}

struct JsonObjectPresence;

impl<'de> Deserialize<'de> for JsonObjectPresence {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ObjectVisitor;

        impl<'de> serde::de::Visitor<'de> for ObjectVisitor {
            type Value = JsonObjectPresence;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a non-null JSON object")
            }

            fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                while map
                    .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(JsonObjectPresence)
            }
        }

        deserializer.deserialize_map(ObjectVisitor)
    }
}

struct JsonArrayPresence;

impl<'de> Deserialize<'de> for JsonArrayPresence {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ArrayVisitor;

        impl<'de> serde::de::Visitor<'de> for ArrayVisitor {
            type Value = JsonArrayPresence;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a non-null JSON array")
            }

            fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                while sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(JsonArrayPresence)
            }
        }

        deserializer.deserialize_seq(ArrayVisitor)
    }
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

    /// Empty-check a response line and enforce the protocol version gate,
    /// returning the decoded prefix (`v` + `ok`). Shared first step for both
    /// the typed dispatch and the raw-history shape validator: the version
    /// gate must run before any `ok` interpretation so an error envelope
    /// advertising an unsupported `v` is still rejected as unsupported.
    fn check_version(&self, line: &str) -> Result<ResponsePrefix> {
        if line.is_empty() {
            bail!("empty response from daemon");
        }
        let prefix: ResponsePrefix = serde_json::from_str(line).context("parsing response JSON")?;
        if prefix.v > SUPPORTED_PROTOCOL_VERSION {
            bail!(
                "whereamid at {} uses unsupported protocol v{}; this client supports through v{}; \
                 upgrade the whereami client or connect to a daemon using supported protocol v{}",
                self.target.description(),
                prefix.v,
                SUPPORTED_PROTOCOL_VERSION,
                SUPPORTED_PROTOCOL_VERSION,
            );
        }
        Ok(prefix)
    }

    /// Parse a daemon response line into the typed response `T`, dispatching on
    /// the `ok` bool (task-0133).
    ///
    /// After the version gate, an `ok:false` line is decoded as
    /// [`ErrorResponse`] and returned as `Err(DaemonError { .. }.into())`; only
    /// an `ok:true` (or `ok`-absent, which then fails `T`'s required `ok`
    /// field) line is decoded into `T`. This replaces the previous in-band
    /// union where every typed struct also carried `error: Option<String>` and
    /// an error envelope coincidence-matched into a fully defaulted `T`.
    ///
    /// Every typed accessor fetches its raw line via the matching `*_raw()`
    /// method (which builds the request struct and calls `raw_command`) and
    /// parses it here. That keeps the typed path and the `--json` passthrough
    /// on a single source of truth for both the request bytes and the
    /// transport (task-0183): they can no longer drift because there is exactly
    /// one place each request is built.
    fn parse_response_line<T>(&self, line: &str) -> Result<T>
    where
        T: for<'de> Deserialize<'de> + ProtocolResponse,
    {
        let prefix = self.check_version(line)?;
        if prefix.ok == Some(false) {
            let envelope: ErrorResponse =
                serde_json::from_str(line).context("parsing daemon error response")?;
            return Err(DaemonError {
                v: envelope.v,
                error: envelope.error,
            }
            .into());
        }
        let response: T = serde_json::from_str(line).context("parsing response JSON")?;
        debug_assert_eq!(response.protocol_version(), prefix.v);
        Ok(response)
    }

    /// Ask "where am I?" based on current stable APs.
    pub fn locate(&self) -> Result<LocateResponse> {
        self.parse_response_line(&self.locate_raw()?)
    }

    /// Raw `locate`: daemon JSON response body after transport framing and
    /// surrounding-whitespace removal; feeds `--json`.
    pub fn locate_raw(&self) -> Result<String> {
        self.raw_command(&serde_json::to_string(&LocateRequest { cmd: "locate" })?)
    }

    /// Look up specific BSSIDs (ephemeral, does not write to cache).
    pub fn resolve(&self, bssids: Vec<String>) -> Result<ResolveResponse> {
        self.parse_response_line(&self.resolve_raw(bssids)?)
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
        self.parse_response_line(&self.scan_raw()?)
    }

    /// Raw `scan`: daemon JSON response body after transport framing and
    /// surrounding-whitespace removal; feeds `--json`.
    pub fn scan_raw(&self) -> Result<String> {
        self.raw_command(&serde_json::to_string(&SimpleRequest { cmd: "scan" })?)
    }

    /// Get cache and API statistics.
    pub fn stats(&self) -> Result<StatsResponse> {
        self.parse_response_line(&self.stats_raw()?)
    }

    /// Raw `stats`: daemon JSON response body after transport framing and
    /// surrounding-whitespace removal; feeds `--json`.
    pub fn stats_raw(&self) -> Result<String> {
        self.raw_command(&serde_json::to_string(&SimpleRequest { cmd: "stats" })?)
    }

    /// Get the daemon's debug snapshot: scan buffer state, per-BSSID
    /// debounce counters, DB classification.
    pub fn debug(&self) -> Result<DebugResponse> {
        self.parse_response_line(&self.debug_raw()?)
    }

    /// Raw `debug`: daemon JSON response body after transport framing and
    /// surrounding-whitespace removal; feeds `--json`.
    pub fn debug_raw(&self) -> Result<String> {
        self.raw_command(&serde_json::to_string(&SimpleRequest { cmd: "debug" })?)
    }

    /// Get the daemon's version string and git revision.
    pub fn version(&self) -> Result<VersionResponse> {
        self.parse_response_line(&self.version_raw()?)
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
        self.parse_response_line(&self.history_raw(range, from, to)?)
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

    /// Query the unsegmented location fixes for a range. `max_fixes` is an
    /// optional daemon-enforced newest-row cap; omission uses the daemon's
    /// default (currently 1000).
    pub fn history_fixes(
        &self,
        range: Option<String>,
        from: Option<String>,
        to: Option<String>,
        max_fixes: Option<u64>,
    ) -> Result<HistoryFixesResponse> {
        self.parse_response_line(&self.history_fixes_raw(range, from, to, max_fixes)?)
    }

    /// Byte-preserving JSON raw-fix history response. Unlike legacy raw
    /// accessors, this opt-in API validates the new success shape so an old
    /// daemon that silently ignores `raw` cannot masquerade as an empty fix
    /// result. A valid line is returned without re-serialization.
    pub fn history_fixes_raw(
        &self,
        range: Option<String>,
        from: Option<String>,
        to: Option<String>,
        max_fixes: Option<u64>,
    ) -> Result<String> {
        let line = self.raw_command(&serde_json::to_string(&HistoryFixesRequest {
            cmd: "history",
            range,
            from,
            to,
            raw: true,
            max_fixes,
        })?)?;
        self.validate_history_fixes_response(&line)?;
        Ok(line)
    }

    fn validate_history_fixes_response(&self, line: &str) -> Result<()> {
        // Non-dispatching, NON-version-gating decode. Two contracts meet here:
        //
        // - task-0133: an `ok:false` error envelope must reach the `--json`
        //   passthrough byte-for-byte, so it decodes here (with `ok` false)
        //   rather than being diverted into a DaemonError.
        // - task-0266: `--json` is a pure daemon transport (task-0183), so the
        //   `--json` path must NOT reject on protocol version for ANY command.
        //   This decode therefore uses plain `serde_json::from_str` instead of
        //   `check_version`, matching the pure `*_raw()` passthrough of
        //   locate/scan: a line advertising `v > SUPPORTED_PROTOCOL_VERSION`
        //   passes through unchanged. Version-gating lives only on the typed
        //   (non-json) path, via `parse_response_line` -> `check_version`.
        //
        // What remains below is a genuine CAPABILITY check (feature support, not
        // protocol version): an old daemon that ignored `raw:true` and returned
        // a SEGMENTED success envelope lacks `range`/`fixes` and is rejected.
        let shape: HistoryFixesShape =
            serde_json::from_str(line).context("parsing raw-history response")?;
        if shape.ok && (shape.range.is_none() || shape.fixes.is_none()) {
            bail!(
                "whereamid returned a successful segmented history response to a raw-fix request; \
                 upgrade whereamid to a version that supports raw history"
            );
        }
        Ok(())
    }

    /// Delete the daemon's persisted and managed in-memory location/AP state.
    /// The explicit enum makes destructive intent visible at every call site.
    pub fn wipe(&self, confirmation: WipeConfirmation) -> Result<WipeResponse> {
        self.parse_response_line(&self.wipe_raw(confirmation)?)
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

// The common version gate needs each response's `v`. task-0133 removed the
// former `DaemonResponse` (is_ok/error) trait: with the `error` field gone
// from the typed structs and `ok:false` now surfaced as a `DaemonError` by
// the dispatch, the ok/error accessors were dead. The CLI reads the daemon
// error off the `DaemonError` in the `Err` arm instead.
macro_rules! impl_protocol_response {
    ($t:ty) => {
        impl ProtocolResponse for $t {
            fn protocol_version(&self) -> u32 {
                self.v
            }
        }
    };
}
impl_protocol_response!(LocateResponse);
impl_protocol_response!(ResolveResponse);
impl_protocol_response!(ScanResponse);
impl_protocol_response!(StatsResponse);
impl_protocol_response!(DebugResponse);
impl_protocol_response!(VersionResponse);
impl_protocol_response!(HistoryResponse);
impl_protocol_response!(HistoryFixesResponse);
impl_protocol_response!(WipeResponse);

#[cfg(test)]
mod history_fixes_wire_tests {
    use super::*;

    fn tcp_daemon_once(response: String) -> (WhereAmIClient, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            stream.read_to_string(&mut request).unwrap();
            // Some shape-only tests do not inspect the captured request; a
            // dropped observer must not prevent this one-shot daemon from
            // writing its response.
            let _ = request_tx.send(request);
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(b"\n").unwrap();
        });
        (WhereAmIClient::tcp(&address.to_string()), request_rx)
    }

    fn client() -> WhereAmIClient {
        WhereAmIClient::unix("/test/whereamid.sock")
    }

    #[test]
    fn legacy_history_request_bytes_remain_exact() {
        assert_eq!(
            serde_json::to_string(&HistoryRequest {
                cmd: "history",
                range: Some("24h".to_string()),
                from: None,
                to: None,
            })
            .unwrap(),
            r#"{"cmd":"history","range":"24h"}"#
        );
    }

    #[test]
    fn raw_history_request_is_explicit_and_carries_the_cap() {
        assert_eq!(
            serde_json::to_string(&HistoryFixesRequest {
                cmd: "history",
                range: Some("24h".to_string()),
                from: None,
                to: None,
                raw: true,
                max_fixes: Some(1_234),
            })
            .unwrap(),
            r#"{"cmd":"history","range":"24h","raw":true,"max_fixes":1234}"#
        );
    }

    #[test]
    fn successful_raw_shape_parses_and_validation_preserves_source_line() {
        let line = r#"{"ok":true,"v":1,"range":{"from_rfc3339":"2026-05-09T00:00:00+00:00","to_rfc3339":"2026-05-10T00:00:00+00:00"},"fixes":[{"at_rfc3339":"2026-05-09T12:00:00+00:00","lat":55.1,"lon":12.2,"accuracy_m":30.0,"n_sources":4}],"elided":2,"future":"preserve"}"#;
        client().validate_history_fixes_response(line).unwrap();
        let response: HistoryFixesResponse = client().parse_response_line(line).unwrap();
        assert_eq!(response.range.from_rfc3339, "2026-05-09T00:00:00+00:00");
        assert_eq!(response.fixes.len(), 1);
        assert_eq!(response.elided, Some(2));
        let legacy = &response.fixes[0];
        assert_eq!(legacy.confidence, None);
        assert_eq!(legacy.implied_speed_kmh, None);
        assert_eq!(legacy.last_fix_age_secs, None);
        assert_eq!(legacy.inlier_ratio, None);
        assert_eq!(legacy.suspect_reason, None);
        assert!(line.contains(r#""future":"preserve""#));
    }

    #[test]
    fn task0245_mixed_quality_rows_are_additive_and_closed_vocabulary() {
        let line = r#"{"ok":true,"v":1,"range":{"from_rfc3339":"a","to_rfc3339":"b"},"fixes":[{"at_rfc3339":"old","lat":1.0,"lon":2.0,"accuracy_m":3.0,"n_sources":4},{"at_rfc3339":"new","lat":5.0,"lon":6.0,"accuracy_m":7.0,"n_sources":8,"implied_speed_kmh":9.5,"last_fix_age_secs":10,"inlier_ratio":0.75,"confidence":"suspect","suspect_reason":"exact"}]}"#;
        let response: HistoryFixesResponse = client().parse_response_line(line).unwrap();
        assert_eq!(response.fixes[0].confidence, None);
        assert_eq!(response.fixes[1].confidence, Some(Confidence::Suspect));
        assert_eq!(response.fixes[1].implied_speed_kmh, Some(9.5));
        assert_eq!(response.fixes[1].last_fix_age_secs, Some(10));
        assert_eq!(response.fixes[1].inlier_ratio, Some(0.75));
        assert_eq!(response.fixes[1].suspect_reason.as_deref(), Some("exact"));

        #[derive(Deserialize)]
        struct LegacyFix {
            at_rfc3339: String,
            lat: f64,
            lon: f64,
            accuracy_m: f64,
            n_sources: i64,
        }
        #[derive(Deserialize)]
        struct LegacyEnvelope {
            fixes: Vec<LegacyFix>,
        }
        let old_client: LegacyEnvelope = serde_json::from_str(line).unwrap();
        assert_eq!(old_client.fixes.len(), 2);
        assert_eq!(old_client.fixes[1].at_rfc3339, "new");
        assert_eq!(old_client.fixes[1].lat, 5.0);
        assert_eq!(old_client.fixes[1].lon, 6.0);
        assert_eq!(old_client.fixes[1].accuracy_m, 7.0);
        assert_eq!(old_client.fixes[1].n_sources, 8);

        let unknown = r#"{"ok":true,"v":1,"range":{"from_rfc3339":"a","to_rfc3339":"b"},"fixes":[{"at_rfc3339":"new","lat":1.0,"lon":2.0,"accuracy_m":3.0,"n_sources":4,"confidence":"future"}]}"#;
        client().validate_history_fixes_response(unknown).unwrap();
        assert!(client()
            .parse_response_line::<HistoryFixesResponse>(unknown)
            .is_err());
        let (raw_client, _) = tcp_daemon_once(unknown.to_string());
        assert_eq!(
            raw_client
                .history_fixes_raw(Some("24h".to_owned()), None, None, None)
                .unwrap(),
            unknown,
            "raw API must preserve unknown confidence bytes"
        );
    }

    #[test]
    fn raw_fix_json_api_preserves_valid_daemon_bytes_and_sends_opt_in_fields() {
        let line = r#"{"v":1, "ok":true, "future":"keep", "fixes":[], "range":{"to_rfc3339":"b","from_rfc3339":"a"}}"#;
        let (client, request_rx) = tcp_daemon_once(line.to_string());
        let returned = client
            .history_fixes_raw(Some("24h".to_string()), None, None, Some(1_234))
            .unwrap();
        assert_eq!(
            returned, line,
            "valid daemon JSON must not be re-serialized"
        );
        assert_eq!(
            request_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap(),
            "{\"cmd\":\"history\",\"range\":\"24h\",\"raw\":true,\"max_fixes\":1234}\n"
        );
    }

    #[test]
    fn typed_raw_fix_api_rejects_old_daemon_segment_success() {
        let (client, _) =
            tcp_daemon_once(r#"{"ok":true,"v":1,"from":"a","to":"b","segments":[]}"#.to_string());
        let error = client
            .history_fixes(Some("24h".to_string()), None, None, None)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("successful segmented history response"),
            "{error}"
        );
    }

    #[test]
    fn raw_fix_json_api_preserves_error_envelopes_without_success_fields() {
        let line = r#"{"v":1, "ok":false, "error":"history: busy; retry later"}"#;
        let (client, _) = tcp_daemon_once(line.to_string());
        assert_eq!(
            client
                .history_fixes_raw(Some("24h".to_string()), None, None, None)
                .unwrap(),
            line
        );
    }

    #[test]
    fn old_daemon_segment_success_is_rejected_but_error_envelope_parses() {
        let old_success = r#"{"ok":true,"v":1,"from":"a","to":"b","segments":[]}"#;
        let error = client()
            .validate_history_fixes_response(old_success)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("successful segmented history response"),
            "{error}"
        );

        // An error envelope still validates (so the raw `--json` path passes it
        // through byte-for-byte), but the TYPED dispatch now surfaces it as a
        // DaemonError carrying the verbatim string rather than a defaulted
        // HistoryFixesResponse with `ok:false` (task-0133).
        let error_line = r#"{"ok":false,"v":1,"error":"history: busy; retry later"}"#;
        client()
            .validate_history_fixes_response(error_line)
            .unwrap();
        let err = client()
            .parse_response_line::<HistoryFixesResponse>(error_line)
            .unwrap_err();
        let daemon = err
            .downcast_ref::<DaemonError>()
            .expect("ok:false must dispatch to a DaemonError");
        assert_eq!(daemon.v, 1);
        assert_eq!(daemon.error, "history: busy; retry later");
    }

    #[test]
    fn successful_raw_response_requires_range_and_fixes_even_when_one_is_empty() {
        for invalid in [
            r#"{"ok":true,"v":1,"fixes":[]}"#,
            r#"{"ok":true,"v":1,"range":{"from_rfc3339":"a","to_rfc3339":"b"}}"#,
            r#"{"ok":true,"v":1,"range":null,"fixes":[]}"#,
            r#"{"ok":true,"v":1,"range":{"from_rfc3339":"a","to_rfc3339":"b"},"fixes":null}"#,
            r#"{"ok":true,"v":1,"range":[],"fixes":[]}"#,
            r#"{"ok":true,"v":1,"range":{"from_rfc3339":"a","to_rfc3339":"b"},"fixes":{}}"#,
        ] {
            assert!(client().validate_history_fixes_response(invalid).is_err());
        }
        client()
            .validate_history_fixes_response(
                r#"{"ok":true,"v":1,"range":{"from_rfc3339":"a","to_rfc3339":"b"},"fixes":[]}"#,
            )
            .unwrap();
    }

    #[test]
    fn presence_validation_skips_fix_payloads_but_typed_parse_remains_strict() {
        let malformed_fix = r#"{"ok":true,"v":1,"range":{"from_rfc3339":"a","to_rfc3339":"b"},"fixes":[{"future_only":true}]}"#;
        client()
            .validate_history_fixes_response(malformed_fix)
            .expect("JSON passthrough validates only the non-null object/array containers");
        assert!(
            client()
                .parse_response_line::<HistoryFixesResponse>(malformed_fix)
                .is_err(),
            "typed history must still validate every HistoryFix field"
        );
    }

    // task-0266: `--json` is a uniform daemon transport. The raw-history path
    // (`history_fixes_raw` -> `validate_history_fixes_response`) must NOT reject
    // a line advertising an unsupported protocol version, matching the pure
    // `*_raw()` passthrough of locate/scan (whose byte-identical forwarding of a
    // `v>SUPPORTED` line is pinned by the `json_locate_passes_newer_protocol_
    // through_byte_identically` integration test in tests/protocol_version.rs).
    #[test]
    fn raw_history_json_passthrough_accepts_unsupported_protocol_version() {
        let unsupported = SUPPORTED_PROTOCOL_VERSION + 1;
        let line = format!(
            r#"{{"ok":true,"v":{unsupported},"range":{{"from_rfc3339":"a","to_rfc3339":"b"}},"fixes":[]}}"#
        );

        // Formerly rejected via decode_versioned -> check_version; now passes.
        client()
            .validate_history_fixes_response(&line)
            .expect("raw-history --json must not version-gate (uniform passthrough)");

        // End-to-end: the byte-preserving accessor forwards the line unchanged.
        let (tcp_client, _rx) = tcp_daemon_once(line.clone());
        assert_eq!(
            tcp_client
                .history_fixes_raw(Some("24h".to_string()), None, None, None)
                .unwrap(),
            line,
            "unsupported-version raw line must pass through byte-identically"
        );
    }

    // task-0266: dropping the version gate must NOT drop the capability check.
    // An old daemon that ignored `raw:true` returns a SEGMENTED envelope (no
    // `range`/`fixes`); that is rejected regardless of protocol version, because
    // it is a FEATURE-support failure, not a protocol-version failure.
    #[test]
    fn raw_history_capability_check_bites_independent_of_protocol_version() {
        let unsupported = SUPPORTED_PROTOCOL_VERSION + 1;
        // A NEWER daemon that still answered with a segmented body is a genuine
        // capability mismatch and must bail, even though its `v` is unsupported
        // (proving the bail is the capability check, not a version rejection).
        let segmented =
            format!(r#"{{"ok":true,"v":{unsupported},"from":"a","to":"b","segments":[]}}"#);
        let error = client()
            .validate_history_fixes_response(&segmented)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("successful segmented history response"),
            "capability check must still fire on a segmented success: {error}"
        );
    }
}

#[cfg(test)]
mod wipe_wire_tests {
    use super::*;

    fn client() -> WhereAmIClient {
        WhereAmIClient::unix("/test/whereamid.sock")
    }

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
        let success: WipeResponse = client().parse_response_line(
            r#"{"ok":true,"v":1,"aps_deleted":5,"pending_deleted":4,"not_found_deleted":3,"fixes_deleted":2,"last_fix_deleted":1}"#,
        )
        .unwrap();
        assert!(success.ok);
        assert_eq!(success.aps_deleted, 5);
        assert_eq!(success.last_fix_deleted, 1);

        // ok:false now surfaces as a typed DaemonError (task-0133), not a
        // WipeResponse with defaulted counters and an in-band error field.
        let err = client()
            .parse_response_line::<WipeResponse>(r#"{"ok":false,"v":1,"error":"wipe refused"}"#)
            .unwrap_err();
        let daemon = err
            .downcast_ref::<DaemonError>()
            .expect("ok:false must dispatch to a DaemonError");
        assert_eq!(daemon.error, "wipe refused");
        // Its Display is exactly the daemon string, so context rendering is
        // unchanged for the CLI.
        assert_eq!(daemon.to_string(), "wipe refused");
    }
}

/// task-0133: the `ok`-dispatch surface — every typed accessor decodes a
/// success line into its response and an `ok:false` line into a `DaemonError`,
/// while the version gate still fires before either. One representative decode
/// per response shape (the daemon fixtures in
/// `whereami-protocol/tests/wire.rs` already pin the full success byte shapes).
#[cfg(test)]
mod daemon_error_dispatch_tests {
    use super::*;

    fn client() -> WhereAmIClient {
        WhereAmIClient::unix("/test/whereamid.sock")
    }

    /// Every response shape's success line decodes into its typed body.
    #[test]
    fn success_lines_decode_into_typed_responses() {
        let l: LocateResponse = client()
            .parse_response_line(r#"{"ok":true,"v":1,"lat":55.5,"lon":12.25,"sources":3}"#)
            .unwrap();
        assert!(l.ok);
        assert_eq!(l.sources, 3);

        let r: ResolveResponse = client()
            .parse_response_line(r#"{"ok":true,"v":1,"results":[]}"#)
            .unwrap();
        assert!(r.results.is_empty());

        let s: ScanResponse = client()
            .parse_response_line(r#"{"ok":true,"v":1,"networks":[],"scan_age_ms":null}"#)
            .unwrap();
        assert!(s.networks.is_empty());

        let st: StatsResponse = client()
            .parse_response_line(r#"{"ok":true,"v":1,"cached_aps":7}"#)
            .unwrap();
        assert_eq!(st.cached_aps, 7);

        let d: DebugResponse = client()
            .parse_response_line(r#"{"ok":true,"v":1,"samples_in_buffer":2}"#)
            .unwrap();
        assert_eq!(d.samples_in_buffer, 2);

        let ver: VersionResponse = client()
            .parse_response_line(r#"{"ok":true,"v":1,"version":"0.1.0","git_rev":"abc"}"#)
            .unwrap();
        assert_eq!(ver.version.as_deref(), Some("0.1.0"));

        let h: HistoryResponse = client()
            .parse_response_line(r#"{"ok":true,"v":1,"from":"a","to":"b","segments":[]}"#)
            .unwrap();
        assert_eq!(h.from, "a");

        let hf: HistoryFixesResponse = client()
            .parse_response_line(
                r#"{"ok":true,"v":1,"range":{"from_rfc3339":"a","to_rfc3339":"b"},"fixes":[]}"#,
            )
            .unwrap();
        assert!(hf.fixes.is_empty());

        let w: WipeResponse = client()
            .parse_response_line(r#"{"ok":true,"v":1,"aps_deleted":1}"#)
            .unwrap();
        assert_eq!(w.aps_deleted, 1);
    }

    /// An `ok:false` envelope dispatches to a DaemonError for EVERY target
    /// response type, carrying the daemon's verbatim string.
    #[test]
    fn error_envelope_dispatches_to_daemon_error_for_each_shape() {
        let line = r#"{"ok":false,"v":1,"error":"unknown command"}"#;
        macro_rules! assert_daemon_error {
            ($ty:ty) => {{
                let err = client().parse_response_line::<$ty>(line).unwrap_err();
                let daemon = err.downcast_ref::<DaemonError>().expect(concat!(
                    stringify!($ty),
                    ": ok:false must yield a DaemonError"
                ));
                assert_eq!(daemon.v, 1);
                assert_eq!(daemon.error, "unknown command");
            }};
        }
        assert_daemon_error!(LocateResponse);
        assert_daemon_error!(ResolveResponse);
        assert_daemon_error!(ScanResponse);
        assert_daemon_error!(StatsResponse);
        assert_daemon_error!(DebugResponse);
        assert_daemon_error!(VersionResponse);
        assert_daemon_error!(HistoryResponse);
        assert_daemon_error!(HistoryFixesResponse);
        assert_daemon_error!(WipeResponse);
    }

    /// The version gate fires before the `ok` dispatch, so an error envelope
    /// advertising an unsupported protocol is rejected as unsupported (not
    /// surfaced as a DaemonError).
    #[test]
    fn version_gate_precedes_ok_dispatch_on_error_envelope() {
        let err = client()
            .parse_response_line::<LocateResponse>(r#"{"ok":false,"v":2,"error":"nope"}"#)
            .unwrap_err();
        assert!(err.downcast_ref::<DaemonError>().is_none());
        assert!(err.to_string().contains("unsupported protocol v2"), "{err}");
    }

    /// An empty line is still a transport-style error, never a DaemonError.
    #[test]
    fn empty_line_is_not_a_daemon_error() {
        let err = client()
            .parse_response_line::<LocateResponse>("")
            .unwrap_err();
        assert!(err.downcast_ref::<DaemonError>().is_none());
        assert_eq!(err.to_string(), "empty response from daemon");
    }
}

#[cfg(test)]
mod protocol_version_tests {
    use super::*;

    #[test]
    fn old_daemon_locate_defaults_new_uncertainty_fields() {
        let response: LocateResponse = WhereAmIClient::tcp("127.0.0.1:4747")
            .parse_response_line(
                r#"{"ok":true,"v":1,"lat":55.6761,"lon":12.5683,"accuracy_m":12.0,"sources":3}"#,
            )
            .unwrap();

        assert_eq!(response.implied_speed_kmh, None);
        assert_eq!(response.last_fix_age_secs, None);
        assert_eq!(response.inlier_ratio, None);
        assert_eq!(response.confidence, Confidence::High);
        assert_eq!(response.suspect_reason, None);
    }

    #[test]
    fn legacy_client_shape_ignores_new_daemon_uncertainty_fields() {
        #[derive(Deserialize)]
        struct LegacyLocate {
            ok: bool,
            v: u32,
            lat: f64,
            lon: f64,
            sources: usize,
        }

        let response: LegacyLocate = serde_json::from_str(
            r#"{"ok":true,"v":1,"lat":55.6761,"lon":12.5683,"sources":3,"implied_speed_kmh":312.5,"last_fix_age_secs":42,"inlier_ratio":0.8,"confidence":"suspect","suspect_reason":"test"}"#,
        )
        .unwrap();
        assert!(response.ok);
        assert_eq!(response.v, 1);
        assert_eq!((response.lat, response.lon), (55.6761, 12.5683));
        assert_eq!(response.sources, 3);
    }

    #[test]
    fn confidence_wire_vocabulary_and_display_are_pinned() {
        for (confidence, wire, display) in [
            (Confidence::High, "\"high\"", "High"),
            (Confidence::Medium, "\"medium\"", "Medium"),
            (Confidence::Low, "\"low\"", "Low"),
            (Confidence::Suspect, "\"suspect\"", "Suspect"),
        ] {
            assert_eq!(serde_json::to_string(&confidence).unwrap(), wire);
            assert_eq!(
                serde_json::from_str::<Confidence>(wire).unwrap(),
                confidence
            );
            assert_eq!(confidence.to_string(), display);
        }
    }

    #[test]
    fn supported_v1_response_parses_normally() {
        let client = WhereAmIClient::tcp("127.0.0.1:4747");
        let response: LocateResponse = client
            .parse_response_line(
                r#"{"ok":true,"v":1,"lat":55.6761,"lon":12.5683,"accuracy_m":12.0,"sources":3}"#,
            )
            .unwrap();

        assert!(response.ok);
        assert_eq!(response.v, 1);
        assert_eq!(response.lat, 55.6761);
        assert_eq!(response.lon, 12.5683);
    }

    #[test]
    fn newer_protocol_is_rejected_before_defaultable_locate_fields_escape() {
        let client = WhereAmIClient::unix("/run/user/1000/whereamid.sock");
        let error = client
            .parse_response_line::<LocateResponse>(r#"{"ok":true,"v":2}"#)
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "whereamid at unix socket /run/user/1000/whereamid.sock uses unsupported protocol v2; \
             this client supports through v1; upgrade the whereami client or connect to a daemon \
             using supported protocol v1"
        );
    }

    #[test]
    fn newer_protocol_error_names_the_exact_tcp_endpoint() {
        let client = WhereAmIClient::tcp("192.0.2.17:8474");
        let error = client
            .parse_response_line::<LocateResponse>(r#"{"ok":true,"v":9}"#)
            .unwrap_err()
            .to_string();

        assert!(error.contains("TCP endpoint 192.0.2.17:8474"), "{error}");
        assert!(error.contains("unsupported protocol v9"), "{error}");
        assert!(error.contains("supports through v1"), "{error}");
        assert!(error.contains("upgrade the whereami client"), "{error}");
    }

    #[test]
    fn additive_unknown_enum_value_remains_compatible_at_v1() {
        let client = WhereAmIClient::tcp("127.0.0.1:4747");
        let response: ResolveResponse = client
            .parse_response_line(
                r#"{"ok":true,"v":1,"results":[{"bssid":"aa:bb:cc:dd:ee:ff","lat":null,"lon":null,"ssid":null,"source":"future_source"}]}"#,
            )
            .unwrap();

        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].provenance, Provenance::Unknown);
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

/// The client read path: `exchange` must return exactly the first response
/// line. The wire-format contract itself is proven in
/// `whereami-protocol/tests/wire.rs` (task-0114); this is the one part of the
/// old cross-suite that exercises client I/O rather than serde, so it stays
/// here.
#[cfg(test)]
mod client_read_path_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    const LOCATE_FIXTURE: &str = r#"{"ok":true,"v":1,"lat":55.6761,"lon":12.5683,"accuracy_m":42.5,"sources":7,"cached":5,"visible":9,"stable":6,"confidence":"medium"}"#;

    /// The peer writes a valid compact line, a newline, then trailing garbage
    /// (as a mistaken multi-line `to_string_pretty` emit would). The client
    /// must return only the first line and decode it cleanly.
    #[test]
    fn client_reads_exactly_the_first_response_line() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let payload = format!("{LOCATE_FIXTURE}\n{{\"ok\":true,\"v\":1}}\nNOT JSON AT ALL");

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("server read timeout");
            let mut request = String::new();
            let _ = stream.read_to_string(&mut request);
            stream.write_all(payload.as_bytes()).expect("write payload");
            stream.flush().expect("flush payload");
        });

        let client = WhereAmIClient::tcp(&addr.to_string());
        let line = client
            .raw_command(r#"{"cmd":"locate"}"#)
            .expect("raw_command should return exactly the first line");
        server.join().expect("server thread");

        assert_eq!(
            line, LOCATE_FIXTURE,
            "client must return exactly the first line, trailing bytes stripped"
        );
        let decoded: LocateResponse =
            serde_json::from_slice(line.as_bytes()).expect("first line decodes on its own");
        assert!(decoded.ok);
        assert_eq!(decoded.confidence, Confidence::Medium);
    }
}
