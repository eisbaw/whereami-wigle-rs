//! whereami-protocol: the single source of truth for the whereamid JSON wire
//! types (task-0114).
//!
//! Every request and response that crosses the daemon/client socket is defined
//! here exactly once. The daemon (`whereamid`) constructs and serializes these
//! types; the client (`whereami-client`) deserializes them. Because both link
//! the same definitions, a field rename, a reordering, or a changed
//! `skip_serializing_if` can no longer drift silently between the two sides —
//! it is one edit, guarded by the round-trip tests in `tests/`.
//!
//! # How the serde attributes were reconciled
//!
//! Historically the two sides diverged deliberately: the daemon used
//! `skip_serializing_if` to omit empty/false fields, while the client used
//! `#[serde(default)]` so an older daemon's missing field decodes cleanly. The
//! two compose, so every field here carries BOTH where each side had one:
//!
//! - A field the daemon always emits (no skip) keeps only `#[serde(default)]`.
//!   `default` never affects serialization, so the daemon's bytes are unchanged;
//!   it only lets an old daemon's absent field decode to the type default.
//! - A field the daemon skipped when empty/false keeps
//!   `#[serde(default, skip_serializing_if = "...")]` with the exact predicate.
//! - The typed response structs carry NO `error` field: the daemon emits
//!   failures exclusively as the dedicated [`ErrorResponse`] envelope
//!   (`{"ok":false,"v":_,"error":_}`), never as an `error` key on a success
//!   struct. The client dispatches on the `ok` bool and decodes an `ok:false`
//!   line as [`ErrorResponse`] (task-0133), so an `error` field on the typed
//!   structs would only be dead weight and an in-band collision hazard. It was
//!   always `error: None` at every daemon construction site, so its removal
//!   keeps success-response bytes byte-identical to today.
//!
//! The wire bytes (field names, casing, order, null-vs-omitted, enum tokens) are
//! intentionally byte-identical to the pre-extraction protocol. This crate is a
//! deduplication, not a wire change.
//!
//! # The deliberate strict/tolerant enum asymmetry (AC#3)
//!
//! [`Provenance`] and [`DbStatus`] each carry a tolerant `Unknown` variant tagged
//! `#[serde(other)]`. This preserves the previous two-type asymmetry with a
//! SINGLE shared type:
//!
//! - The **daemon** is strict *by construction*: it only ever constructs the
//!   known variants (`Api`/`Cache`/`NotFound`/`Unresolved`, and
//!   `Cached`/`Pending`/`NotFound`/`New`) and never `Unknown`, so its emitted
//!   JSON is unchanged and never contains an unknown token.
//! - The **client** is tolerant on decode: an unrecognized wire token from a
//!   future daemon maps to `Unknown` instead of failing the whole response, so
//!   an older client keeps parsing (forward-compat escape hatch).
//!
//! [`Confidence`] has a different, milder asymmetry: it derives `Default` with
//! `#[default] High` so an old daemon's missing `locate.confidence` field decodes
//! to the historic optimistic `High`. History uses `Option<Confidence>`, where
//! absence is legacy-unknown (`None`), never `High`.

use serde::{Deserialize, Serialize};

// =====================================================================
// Shared wire enums
// =====================================================================

/// Plausibility confidence attached to every successful `locate` response and
/// to each persisted history fix.
///
/// The wire vocabulary is the lowercase token (`"high"`, `"medium"`, `"low"`,
/// `"suspect"`). `Default` is `High`: an older daemon predates explicit
/// uncertainty reporting, so a missing `locate.confidence` decodes to the
/// historic optimistic value. History deliberately uses `Option<Confidence>`
/// instead, where absence is legacy-unknown (`None`), never `High`.
///
/// Adding a variant is a wire-protocol and DB-schema compatibility change, not a
/// display-only edit.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// Backward-compatible default for responses from older daemons, which
    /// predate explicit uncertainty reporting.
    #[default]
    High,
    Medium,
    Low,
    Suspect,
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Confidence::High => "High",
            Confidence::Medium => "Medium",
            Confidence::Low => "Low",
            Confidence::Suspect => "Suspect",
        })
    }
}

/// Provenance of a single `resolve` result (task-0062).
///
/// Distinct from the data-`Source` enum (apple/wigle/beacondb/manual/unknown)
/// which records WHO supplied the position. Provenance records HOW the daemon
/// obtained it for *this request*. The wire field name is kept as `source` for
/// backward compatibility with CLIs and curl scripts that parsed the prior
/// stringly-typed protocol.
///
/// The daemon never constructs [`Provenance::Unknown`]; the client maps an
/// unrecognized future token to it (see the module-level asymmetry note).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Fetched via a remote provider during this resolve call.
    Api,
    /// Served from the local SQLite cache.
    Cache,
    /// Authoritative absence: a provider returned NotFound for this BSSID on
    /// this call, or the not_found table recorded absence within its TTL. Safe
    /// for a client to cache as "not locatable".
    NotFound,
    /// task-0194: the daemon could not consult a provider for this BSSID this
    /// call (deferred to an in-flight resolution, provider skipped for missing
    /// creds / exhausted or corrupt quota, or a transient failure), so absence
    /// is NOT established. Distinct from NotFound so clients that cache
    /// not_found don't blacklist a still-resolvable BSSID; retry later.
    Unresolved,
    /// A value a future daemon emitted that this client version does not
    /// recognise. Forward-compat escape hatch; never constructed by the daemon.
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

/// DB classification of a BSSID seen by the debouncer (task-0062).
///
/// Wire format mirrors the daemon's snake_case strings. The daemon never
/// constructs [`DbStatus::Unknown`]; the client maps an unrecognized future
/// token to it (see the module-level asymmetry note).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DbStatus {
    /// BSSID is in the `aps` cache.
    Cached,
    /// BSSID is queued for resolution.
    Pending,
    /// BSSID is in the `not_found` table (definitively absent within TTL).
    NotFound,
    /// BSSID has not been observed by the daemon before this scan.
    New,
    /// A value a future daemon emitted that this client version does not
    /// recognise. Forward-compat escape hatch; never constructed by the daemon.
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

// =====================================================================
// Request types (client -> daemon)
//
// These are serialized by the client. The daemon parses incoming requests with
// its own internal `Request` enum (`#[serde(tag = "cmd")]`), which stays in the
// daemon; these structs document and pin the exact request bytes the client
// emits. They are outbound-only, so they derive `Serialize` (not `Deserialize`:
// the `cmd: &'static str` field cannot be deserialized into a borrowed static).
// =====================================================================

/// `locate` request: `{"cmd":"locate"}`.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct LocateRequest {
    pub cmd: &'static str,
}

/// A request carrying only a command tag (`scan`, `stats`, `debug`, `version`).
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct SimpleRequest {
    pub cmd: &'static str,
}

/// `resolve` request with the explicit BSSID list.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct ResolveRequest {
    pub cmd: &'static str,
    pub bssids: Vec<String>,
}

/// Segmented `history` request. Exactly one of `range` or (`from`,`to`) is set;
/// each absent field is omitted from the wire.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct HistoryRequest {
    pub cmd: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

/// Raw-fix `history` request: sets `raw:true` and an optional newest-row cap.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct HistoryFixesRequest {
    pub cmd: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub raw: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fixes: Option<u64>,
}

// =====================================================================
// Response types (daemon -> client)
// =====================================================================

/// Generic error envelope the daemon returns for any failed command
/// (`{"ok":false,"v":<version>,"error":"..."}`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ErrorResponse {
    pub ok: bool,
    pub v: u32,
    pub error: String,
}

/// Response from the `locate` command.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LocateResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub lat: f64,
    #[serde(default)]
    pub lon: f64,
    /// Weighted spread of contributing AP positions around the centroid,
    /// clamped to [10, 1000] m; a rough dispersion estimate, NOT a calibrated
    /// 68%/95% confidence radius. When `fallback` is true this is the 1000 m
    /// antipodal-ambiguity sentinel rather than a measured spread.
    #[serde(default)]
    pub accuracy_m: f64,
    #[serde(default)]
    pub sources: usize,
    #[serde(default)]
    pub cached: usize,
    #[serde(default)]
    pub visible: usize,
    #[serde(default)]
    pub stable: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// True when the daemon could not produce a current fix and is returning
    /// the previous known position. Emitted only when true; absence decodes to
    /// false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
    /// True iff the reported position is the antipodal-cancellation fallback
    /// (the centroid was ambiguous, so the strongest AP was used and
    /// `accuracy_m` is the 1000 m sentinel). Emitted only when true, mirroring
    /// `stale`; absence decodes to false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fallback: bool,
    /// Age of the stale fix, in seconds. Only meaningful when `stale`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_s: Option<u64>,
    /// Implied speed from the previous accepted fix, absent on cold boot or
    /// when returning the generic stale fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implied_speed_kmh: Option<f64>,
    /// Age of the previous accepted fix used by the plausibility check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fix_age_secs: Option<i64>,
    /// Consensus fraction for the selected winner, absent with fewer than three
    /// eligible visible cached APs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inlier_ratio: Option<f64>,
    /// Older daemons omit confidence; that omission decodes to `High`.
    #[serde(default)]
    pub confidence: Confidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspect_reason: Option<String>,
}

/// A single result from the `resolve` command.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ResolveResultEntry {
    pub bssid: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub ssid: Option<String>,
    /// Wire field name stays `source` for backward compat; the typed field is
    /// `provenance` to disambiguate from the data-source `Source` enum
    /// (task-0062). Emitted with no `skip_serializing_if`, so `lat`/`lon`/`ssid`
    /// serialize as JSON `null` when absent.
    #[serde(rename = "source")]
    pub provenance: Provenance,
}

/// Response from the `resolve` command.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ResolveResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub results: Vec<ResolveResultEntry>,
}

/// A single network from the `scan` command.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct NetworkEntry {
    pub bssid: String,
    pub ssid: Option<String>,
    pub signal_dbm: i32,
    pub channel: Option<i32>,
}

/// Response from the `scan` command.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ScanResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub networks: Vec<NetworkEntry>,
    /// Age of the most recent scan in milliseconds. Serialized as `null` (no
    /// `skip_serializing_if`) when no scan has completed yet.
    #[serde(default)]
    pub scan_age_ms: Option<u64>,
    /// RFC 3339 timestamp of the most recent scan, if any. Omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanned_at: Option<String>,
}

/// One BSSID's debug state, as the daemon returns it under `bssids`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DebugBssid {
    pub bssid: String,
    /// `null` when the BSSID is debounce-stable but absent from the most recent
    /// scan (was previously fabricated as -90 dBm; task-0051).
    #[serde(default)]
    pub signal_dbm: Option<i32>,
    pub seen: usize,
    pub needed: usize,
    pub is_stable: bool,
    pub db_status: DbStatus,
}

/// Response from the `debug` command.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
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
}

/// Response from the `version` command.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct VersionResponse {
    pub ok: bool,
    pub v: u32,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub git_rev: Option<String>,
}

/// One stay-point segment in a `history` response.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct HistorySegment {
    /// First fix in the segment (RFC3339).
    pub start_rfc3339: String,
    /// Last fix in the segment (RFC3339).
    pub end_rfc3339: String,
    /// Duration in seconds.
    pub duration_secs: i64,
    /// Centroid latitude (geometric median of fix latitudes — task-0084).
    pub centroid_lat: f64,
    /// Centroid longitude (geometric median of fix longitudes — task-0084).
    pub centroid_lon: f64,
    /// Mean accuracy_m of fixes in the segment.
    pub mean_accuracy_m: f64,
    /// Number of fixes aggregated.
    pub n_fixes: usize,
}

/// Response from the segmented `history` command.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct HistoryResponse {
    pub ok: bool,
    pub v: u32,
    /// RFC3339 inclusive start of the queried range.
    #[serde(default)]
    pub from: String,
    /// RFC3339 inclusive end of the queried range.
    #[serde(default)]
    pub to: String,
    /// Stay-point segments in ascending time order.
    #[serde(default)]
    pub segments: Vec<HistorySegment>,
}

/// Normalized inclusive bounds returned by raw history.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
pub struct HistoryRange {
    pub from_rfc3339: String,
    pub to_rfc3339: String,
}

/// One unsegmented location-history observation.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct HistoryFix {
    pub at_rfc3339: String,
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: f64,
    pub n_sources: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implied_speed_kmh: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fix_age_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inlier_ratio: Option<f64>,
    /// Unlike locate responses, absent history confidence is legacy unknown and
    /// must never default to `High`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspect_reason: Option<String>,
}

/// Response from an opt-in raw-fix `history` request.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct HistoryFixesResponse {
    pub ok: bool,
    pub v: u32,
    /// Empty only on an `ok:false` error envelope.
    #[serde(default)]
    pub range: HistoryRange,
    /// Empty only when the successful range contains no fixes, or on error.
    #[serde(default)]
    pub fixes: Vec<HistoryFix>,
    /// Exact number of matching earlier rows omitted by the requested cap.
    /// Omitted from the wire when absent (the daemon skips it), matching the
    /// pre-extraction raw-history response shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elided: Option<i64>,
}

/// Response from the `stats` command.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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
    /// In-flight provider lookups currently coalescing concurrent requests
    /// (task-0074). Defaults to 0 for older daemons that don't emit it.
    #[serde(default)]
    pub inflight: usize,
    /// Cumulative count of best-effort DB writes that failed since daemon start
    /// (task-0076). Defaults to 0 for older daemons.
    #[serde(default)]
    pub db_write_failures: u64,
    /// Cumulative count of best-effort DB reads (cache lookups) that failed
    /// since daemon start (task-0105). Distinguishes a real read error from a
    /// genuine cache miss. Defaults to 0 for older daemons.
    #[serde(default)]
    pub db_read_failures: u64,
    /// Successfully committed autonomous history rows since daemon start.
    /// Defaults to 0 for older daemons.
    #[serde(default)]
    pub autolocate_fixes_written: u64,
    /// Background tasks force-aborted at the end of the LAST clean daemon
    /// shutdown. One persisted shutdown snapshot, neither a current count nor a
    /// cumulative total; a crash or SIGKILL does not update it. Defaults to 0
    /// for older daemons (task-0102).
    #[serde(default)]
    pub orphan_tasks_aborted: u64,
}

/// Response from a confirmed privacy wipe.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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
}
