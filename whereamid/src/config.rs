//! CLI arg parsing (clap) and TOML config file loading for secrets.

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::fs::{File, Metadata};
use std::io::{ErrorKind, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::scanner::DEFAULT_MOBILE_SSID_DENYLIST;

/// Portable operational ceiling for active control-channel connections.
/// Deliberately far below Tokio's target-dependent semaphore limit on every
/// supported target, while still allowing unusually busy local deployments.
pub const MAX_CONNECTIONS: usize = 4096;
const _: () = assert!(MAX_CONNECTIONS <= tokio::sync::Semaphore::MAX_PERMITS);

/// Operational ceiling for autonomous collection cadence: one year. This
/// keeps Tokio deadline construction comfortably representable while still
/// allowing an effectively dormant enabled loop. Zero remains the off-switch.
pub const MAX_AUTOLOCATE_INTERVAL_SECS: u64 = 31_536_000;

/// Operational ceiling for the autonomous scan-freshness threshold. Zero is
/// an explicit legacy opt-out; positive values are bounded to one year so the
/// Rust and Nix configuration surfaces share a finite, portable range.
pub const MAX_AUTOLOCATE_SCAN_AGE_SECS: u64 = 31_536_000;

/// whereamid: Wi-Fi geolocation daemon
#[derive(Parser, Debug, Clone)]
#[command(name = "whereamid", version, about)]
pub struct Args {
    /// Unix domain socket path for the control channel (the canonical
    /// transport). When unset, the path is computed at startup:
    /// `$XDG_RUNTIME_DIR/whereamid.sock`, or `/run/whereami/whereamid.sock`
    /// if `XDG_RUNTIME_DIR` is unset. Access is gated by filesystem
    /// permissions (0600 when the daemon self-binds). task-0196.
    #[arg(long)]
    pub socket: Option<String>,

    /// TCP bind address (`host:port`). OPT-IN: when unset the daemon serves
    /// only on the unix socket. TCP has NO peer authentication — every local
    /// user (or, for a non-loopback address, every reachable host) can read
    /// location history / live position / Wi-Fi scans and drive WiGLE quota.
    /// A non-loopback address additionally requires `--allow-nonlocal-bind`.
    /// task-0196.
    #[arg(long)]
    pub bind: Option<String>,

    /// Permit binding `--bind` to a non-loopback address. Without it a
    /// non-loopback `--bind` is rejected at startup, because it exposes the
    /// unauthenticated daemon to other hosts. task-0196.
    #[arg(long)]
    pub allow_nonlocal_bind: bool,

    /// Path to SQLite database
    #[arg(long, default_value = "/var/lib/whereami/aps.sqlite")]
    pub db: PathBuf,

    /// Wi-Fi interface name
    #[arg(long, default_value = "wlan0")]
    pub interface: String,

    /// Path to TOML WiGLE API credentials file. Keep ordinary files owner-only
    /// (0600); systemd's protected $CREDENTIALS_DIRECTORY/config copy is the
    /// only supported exception.
    #[arg(long, default_value = "~/.config/whereami/config.toml")]
    pub config: String,

    /// Maximum number of concurrently active control-channel connections
    /// (1..=4096), shared by the unix socket and optional TCP listener. Excess
    /// clients remain in the kernel listen backlog until capacity is available.
    #[arg(long, default_value_t = 32)]
    pub max_connections: usize,

    /// Scan interval during fast phase (seconds)
    #[arg(long, default_value_t = 10)]
    pub scan_interval_fast: u64,

    /// How long the fast scan phase lasts (seconds)
    #[arg(long, default_value_t = 60)]
    pub scan_fast_duration: u64,

    /// Scan interval during steady phase (seconds)
    #[arg(long, default_value_t = 60)]
    pub scan_interval_slow: u64,

    /// Seconds between autonomous cache-only location fixes
    /// (0..=31536000). The first fix is delayed by one interval. 0 disables
    /// autonomous location history.
    #[arg(
        long,
        default_value_t = 120,
        value_parser = clap::value_parser!(u64).range(0..=MAX_AUTOLOCATE_INTERVAL_SECS)
    )]
    pub autolocate_interval_secs: u64,

    /// Maximum age in seconds of a confirmed radio refresh used by autonomous
    /// location history (0..=31536000). Equality is fresh. 0 accepts any
    /// present snapshot, including unconfirmed cached scanner output.
    #[arg(
        long,
        default_value_t = 180,
        value_parser = clap::value_parser!(u64).range(0..=MAX_AUTOLOCATE_SCAN_AGE_SECS)
    )]
    pub autolocate_max_scan_age_secs: u64,

    /// Number of scan samples in debounce ring buffer
    #[arg(long, default_value_t = 10)]
    pub debounce_window: usize,

    /// Minimum appearances in ring buffer to be considered stable
    #[arg(long, default_value_t = 5)]
    pub debounce_threshold: usize,

    /// Only consider top-N strongest APs for trilateration
    #[arg(long, default_value_t = 10)]
    pub top_n: usize,

    /// Maximum plausible travel speed between the previous accepted fix and
    /// the current trilateration result, in kilometers per hour.
    #[arg(long, default_value_t = 250.0)]
    pub max_travel_speed_kmh: f64,

    /// Comma-separated anchored `*`/`?` globs for scanner-observed SSIDs that
    /// identify mobile access points. Matching uses locale-independent Unicode
    /// lowercase comparison. An empty value disables the heuristic;
    /// missing/empty/`<hidden>` SSIDs never match.
    #[arg(
        long,
        value_delimiter = ',',
        default_value = DEFAULT_MOBILE_SSID_DENYLIST
    )]
    pub mobile_ssid_denylist: Vec<String>,

    /// Seconds between pending queue drain runs
    #[arg(long, default_value_t = 300)]
    pub pending_interval: u64,

    /// Drop from pending after this many failed attempts
    #[arg(long, default_value_t = 20)]
    pub pending_max_attempts: i32,

    /// Maximum WiGLE lookups per day. We optimistically mirror WiGLE's
    /// server-side daily counter to enforce this ceiling; drift is possible if
    /// other clients share the same WiGLE credentials, so it is a best-effort
    /// guard rather than an authoritative balance. The legacy `--daily-limit`
    /// spelling is accepted as a deprecated alias for one release (task-0130).
    #[arg(
        long = "wigle-daily-limit",
        alias = "daily-limit",
        default_value_t = 100
    )]
    pub wigle_daily_limit: u32,

    /// Days before re-checking a not-found BSSID
    #[arg(long, default_value_t = 30)]
    pub not_found_ttl_days: i64,

    /// Include approximate street address in locate responses (via OSM Nominatim)
    #[arg(long)]
    pub address_approx: bool,

    /// TTL for cached reverse-geocoded addresses (days). After this, an
    /// entry is re-resolved on the next locate at that grid cell.
    #[arg(long, default_value_t = 7)]
    pub address_cache_ttl_days: i64,

    /// Total HTTP request timeout for fast endpoints (Apple WPS, WiGLE) in seconds.
    /// Connect timeout is fixed at 5s.
    #[arg(long, default_value_t = 15)]
    pub http_timeout_secs: u64,

    /// Total HTTP request timeout for the OSM Nominatim endpoint in seconds.
    /// Nominatim's public instance is frequently slow under load.
    #[arg(long, default_value_t = 30)]
    pub nominatim_timeout_secs: u64,

    /// Days to retain rows in the location-history `fixes` table. Older rows
    /// are pruned periodically. 0 disables retention (no pruning); fixes are
    /// kept indefinitely.
    #[arg(
        long,
        default_value_t = 30,
        value_parser = clap::value_parser!(i64).range(0..=crate::db::MAX_RETENTION_DAYS)
    )]
    pub history_retention_days: i64,

    /// Days to retain resolved access points after their last observation.
    /// 0 disables retention (cached AP rows are kept indefinitely).
    #[arg(
        long,
        default_value_t = 30,
        value_parser = clap::value_parser!(i64).range(0..=crate::db::MAX_RETENTION_DAYS)
    )]
    pub aps_retention_days: i64,

    /// Maximum distance (meters) between consecutive fixes for them to be
    /// grouped into the same stay-point segment.
    #[arg(long, default_value_t = 100)]
    pub history_segment_distance_m: u64,

    /// Minimum duration (seconds) for a segment to be reported. Shorter
    /// runs (e.g. driving past a coffee shop) are dropped.
    #[arg(long, default_value_t = 300)]
    pub history_segment_min_duration_secs: u64,
}

impl Args {
    /// Cross-field validation that clap cannot express in a single
    /// `value_parser`. Call immediately after `parse()` so we fail fast
    /// before any side effects (DB open, signals, threads, etc.).
    ///
    /// Single-field bounds (e.g. > 0) live on the field's `value_parser`
    /// where clap can render the help text; this method is for invariants
    /// across multiple fields.
    pub fn validate(&self) -> Result<()> {
        // --bind is opt-in TCP. Parse it here (fail fast on garbage — the
        // field used to be a raw String that silently accepted any junk) and
        // require an explicit opt-in for any non-loopback address. A
        // non-loopback bind exposes location history, live position, Wi-Fi
        // scans and WiGLE quota to every host that can reach it, over a
        // transport with no peer authentication (task-0196).
        if let Some(addr) = self.parsed_bind()? {
            if !addr.ip().is_loopback() && !self.allow_nonlocal_bind {
                anyhow::bail!(
                    "--bind {addr} is a non-loopback address; TCP has no peer authentication, so \
                     this exposes location history, live position, Wi-Fi scans and WiGLE quota to \
                     every host that can reach it. Pass --allow-nonlocal-bind to confirm."
                );
            }
        } else if self.allow_nonlocal_bind {
            // Silent no-ops are how misconfigurations hide: the operator who
            // passes this flag believes TCP is on (task-0196 review, mped L3).
            anyhow::bail!("--allow-nonlocal-bind has no effect without --bind");
        }
        if let Some(socket) = &self.socket {
            if socket.trim().is_empty() {
                anyhow::bail!("--socket must not be empty");
            }
        }
        if self.max_connections == 0 {
            anyhow::bail!("--max-connections must be > 0");
        }
        if self.max_connections > MAX_CONNECTIONS {
            anyhow::bail!(
                "--max-connections ({}) must be <= {}",
                self.max_connections,
                MAX_CONNECTIONS
            );
        }
        if self.debounce_window == 0 {
            anyhow::bail!("--debounce-window must be > 0");
        }
        if self.debounce_threshold == 0 {
            anyhow::bail!("--debounce-threshold must be > 0");
        }
        if self.debounce_threshold > self.debounce_window {
            anyhow::bail!(
                "--debounce-threshold ({}) must be <= --debounce-window ({})",
                self.debounce_threshold,
                self.debounce_window
            );
        }
        if self.scan_interval_fast == 0 {
            anyhow::bail!("--scan-interval-fast must be > 0");
        }
        if self.scan_interval_slow == 0 {
            anyhow::bail!("--scan-interval-slow must be > 0");
        }
        if self.autolocate_interval_secs > MAX_AUTOLOCATE_INTERVAL_SECS {
            anyhow::bail!(
                "--autolocate-interval-secs must be in 0..={} (0 disables autonomous location history)",
                MAX_AUTOLOCATE_INTERVAL_SECS
            );
        }
        if self.autolocate_max_scan_age_secs > MAX_AUTOLOCATE_SCAN_AGE_SECS {
            anyhow::bail!(
                "--autolocate-max-scan-age-secs must be in 0..={} (0 accepts legacy unconfirmed/stale snapshots)",
                MAX_AUTOLOCATE_SCAN_AGE_SECS
            );
        }
        if self.top_n == 0 {
            anyhow::bail!("--top-n must be > 0");
        }
        if !self.max_travel_speed_kmh.is_finite() || self.max_travel_speed_kmh <= 0.0 {
            anyhow::bail!("--max-travel-speed-kmh must be a finite value > 0");
        }
        if self
            .mobile_ssid_denylist
            .iter()
            .any(|pattern| pattern.contains('\n') || pattern.contains('\r'))
        {
            anyhow::bail!(
                "--mobile-ssid-denylist patterns must not contain carriage returns or newlines"
            );
        }
        if self.pending_interval == 0 {
            anyhow::bail!("--pending-interval must be > 0");
        }
        if self.pending_max_attempts <= 0 {
            anyhow::bail!("--pending-max-attempts must be > 0");
        }
        if self.not_found_ttl_days <= 0 {
            anyhow::bail!("--not-found-ttl-days must be > 0");
        }
        if self.address_cache_ttl_days < 0 {
            anyhow::bail!("--address-cache-ttl-days must be >= 0 (0 disables the cache)");
        }
        if self.http_timeout_secs == 0 {
            anyhow::bail!("--http-timeout-secs must be > 0");
        }
        if self.nominatim_timeout_secs == 0 {
            anyhow::bail!("--nominatim-timeout-secs must be > 0");
        }
        if !(0..=crate::db::MAX_RETENTION_DAYS).contains(&self.history_retention_days) {
            anyhow::bail!(
                "--history-retention-days must be in 0..={} (0 disables retention)",
                crate::db::MAX_RETENTION_DAYS
            );
        }
        if !(0..=crate::db::MAX_RETENTION_DAYS).contains(&self.aps_retention_days) {
            anyhow::bail!(
                "--aps-retention-days must be in 0..={} (0 disables retention)",
                crate::db::MAX_RETENTION_DAYS
            );
        }
        if self.history_segment_distance_m == 0 {
            anyhow::bail!("--history-segment-distance-m must be > 0");
        }
        if self.history_segment_min_duration_secs == 0 {
            anyhow::bail!("--history-segment-min-duration-secs must be > 0");
        }
        Ok(())
    }

    /// Parse `--bind` into a `SocketAddr`, or `None` when TCP is disabled (the
    /// default: the daemon serves only on the unix socket). Single source of
    /// truth for the bind string so `validate()` and `run_server` cannot drift
    /// on how the address is interpreted. Errors if a bind string was given
    /// but does not parse (task-0196).
    pub fn parsed_bind(&self) -> Result<Option<std::net::SocketAddr>> {
        match &self.bind {
            None => Ok(None),
            Some(s) => {
                let addr: std::net::SocketAddr = s.parse().with_context(|| {
                    format!("--bind {s:?} is not a valid host:port socket address")
                })?;
                Ok(Some(addr))
            }
        }
    }
}

/// TOML config file structure (secrets only).
#[derive(Deserialize, Debug, Clone, Default)]
pub struct ConfigFile {
    #[serde(default)]
    pub wigle: WigleConfig,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct WigleConfig {
    #[serde(default)]
    pub api_user: String,
    #[serde(default)]
    pub api_key: String,
}

/// Expand ~ to home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Was the deprecated `--daily-limit` spelling used on the command line?
///
/// clap's `alias` accepts both `--daily-limit` and `--wigle-daily-limit` but
/// does NOT report which one the user typed, so we scan the raw argv ourselves
/// to decide whether to emit a one-line deprecation notice. Matches both the
/// space form (`--daily-limit 50`) and the `=` form (`--daily-limit=50`). Pure
/// and side-effect-free so it can be unit-tested directly (task-0130).
pub fn daily_limit_alias_used<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|arg| {
        let arg = arg.as_ref();
        arg == "--daily-limit" || arg.starts_with("--daily-limit=")
    })
}

/// Emit a one-line deprecation notice to stderr if the process was invoked with
/// the deprecated `--daily-limit` spelling. Called once at startup, after arg
/// parsing (the alias still parses; this only nudges users to the new name).
pub fn warn_if_deprecated_daily_limit() {
    if daily_limit_alias_used(std::env::args()) {
        eprintln!("warning: --daily-limit is deprecated; use --wigle-daily-limit");
    }
}

/// Load the TOML config file. Returns default if file does not exist.
pub fn load_config_file(path: &str) -> Result<ConfigFile> {
    let credentials_directory = std::env::var_os("CREDENTIALS_DIRECTORY").map(PathBuf::from);
    load_config_file_with_credentials_directory(path, credentials_directory.as_deref())
}

/// Whether `path` is exactly the config credential delivered to this process
/// by systemd. A nonempty absolute directory is required: relative, empty,
/// sibling, and prefix paths must retain the ordinary owner-only mode check.
fn is_systemd_config_credential(path: &Path, directory: Option<&Path>) -> bool {
    let Some(directory) = directory else {
        return false;
    };
    !directory.as_os_str().is_empty() && directory.is_absolute() && path == directory.join("config")
}

/// Verify that the configured path still names the regular file represented by
/// the already-open descriptor metadata. `symlink_metadata` inspects the entry
/// rather than following it. Device/inode equality detects a path replacement
/// after open; inspection failure and every mismatch fail closed.
fn direct_entry_identifies_open_file(path: &Path, opened: &Metadata) -> bool {
    let Ok(entry) = std::fs::symlink_metadata(path) else {
        return false;
    };
    entry.file_type().is_file() && entry.dev() == opened.dev() && entry.ino() == opened.ino()
}

/// Injectable loader core. Tests pass the credential directory explicitly so
/// parallel tests never mutate process-global CREDENTIALS_DIRECTORY.
fn load_config_file_with_credentials_directory(
    path: &str,
    credentials_directory: Option<&Path>,
) -> Result<ConfigFile> {
    let expanded = expand_tilde(path);
    // Open once, then inspect and read that exact descriptor. A separate
    // exists()/metadata()/read_to_string sequence would let a path be swapped
    // between the permission check and the read.
    let mut file = match File::open(&expanded) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            tracing::warn!(
                "config file not found at {}, using defaults",
                expanded.display()
            );
            return Ok(ConfigFile::default());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("opening config file {}", expanded.display()));
        }
    };

    match file.metadata() {
        Ok(metadata) => {
            let mode = metadata.permissions().mode() & 0o777;
            // systemd LoadCredential protects its per-service credential
            // directory and exposes the supported config copy as 0440. That
            // group bit does not make the file available to arbitrary local
            // users. Keep the exception deliberately exact and reject an
            // exact-name symlink or post-open path replacement: the direct
            // path entry must be the same regular file as this descriptor.
            let protected_systemd_copy = mode == 0o440
                && is_systemd_config_credential(&expanded, credentials_directory)
                && direct_entry_identifies_open_file(&expanded, &metadata);
            if mode & 0o077 != 0 && !protected_systemd_copy {
                tracing::warn!(
                    "WiGLE credentials file {:?} has permissions {:04o}; another local user may \
                     read or modify the WiGLE API credentials. Restrict it to owner-only mode \
                     0600.",
                    expanded,
                    mode
                );
            }
        }
        Err(error) => {
            // Permission inspection is advisory: never make an otherwise
            // readable credentials file unusable because metadata failed.
            tracing::warn!(
                "cannot verify permissions for WiGLE credentials file {:?}: {error}; another local \
                 user may be able to read or modify the WiGLE API credentials. Restrict it to \
                 owner-only mode 0600.",
                expanded
            );
        }
    }

    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .with_context(|| format!("reading config file {}", expanded.display()))?;
    let config: ConfigFile = toml::from_str(&contents)
        .with_context(|| format!("parsing config file {}", expanded.display()))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    const TEST_API_KEY: &str = "unit-secret-that-must-never-be-logged";

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "whereamid-config-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("create config test directory");
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_config(path: &std::path::Path, mode: u32) {
        std::fs::write(
            path,
            format!("[wigle]\napi_user = \"unit-user\"\napi_key = \"{TEST_API_KEY}\"\n"),
        )
        .expect("write config test file");
        let mut permissions = std::fs::metadata(path)
            .expect("stat config test file")
            .permissions();
        permissions.set_mode(mode);
        std::fs::set_permissions(path, permissions).expect("set config test file permissions");
    }

    fn load_with_captured_events(
        path: &std::path::Path,
    ) -> (ConfigFile, Vec<(tracing::Level, String)>) {
        load_with_captured_events_and_credentials_directory(path, None)
    }

    fn load_with_captured_events_and_credentials_directory(
        path: &std::path::Path,
        credentials_directory: Option<&std::path::Path>,
    ) -> (ConfigFile, Vec<(tracing::Level, String)>) {
        // task-0255: shared capture sink; this site asserts on (level,
        // message) only, so drop the unified sink's `target` column.
        let (config, events) = crate::test_support::capture(|| {
            load_config_file_with_credentials_directory(
                path.to_str().expect("UTF-8 config test path"),
                credentials_directory,
            )
        });
        let config = config.expect("load config test file");
        let events = events
            .into_iter()
            .map(|(level, _target, message)| (level, message))
            .collect();
        (config, events)
    }

    fn rendered_events(events: &[(tracing::Level, String)]) -> String {
        events
            .iter()
            .map(|(level, message)| format!("{level}: {message}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn parse(extra: &[&str]) -> Args {
        let mut argv = vec!["whereamid"];
        argv.extend_from_slice(extra);
        Args::parse_from(argv)
    }

    #[test]
    fn wigle_daily_limit_defaults_and_parses_new_spelling() {
        assert_eq!(parse(&[]).wigle_daily_limit, 100);
        assert_eq!(parse(&["--wigle-daily-limit", "50"]).wigle_daily_limit, 50);
        assert_eq!(parse(&["--wigle-daily-limit=7"]).wigle_daily_limit, 7);
    }

    #[test]
    fn deprecated_daily_limit_alias_still_parses_to_wigle_daily_limit() {
        // task-0130: --daily-limit remains accepted for one release via clap
        // alias, mapping to the same field as --wigle-daily-limit.
        assert_eq!(parse(&["--daily-limit", "42"]).wigle_daily_limit, 42);
        assert_eq!(parse(&["--daily-limit=9"]).wigle_daily_limit, 9);
    }

    #[test]
    fn daily_limit_alias_used_detects_only_the_deprecated_spelling() {
        // Space form and = form of the deprecated flag are detected.
        assert!(daily_limit_alias_used(["--daily-limit", "50"]));
        assert!(daily_limit_alias_used(["--daily-limit=50"]));
        // The new spelling and unrelated flags are NOT flagged as deprecated.
        assert!(!daily_limit_alias_used(["--wigle-daily-limit", "50"]));
        assert!(!daily_limit_alias_used(["--wigle-daily-limit=50"]));
        assert!(!daily_limit_alias_used(["--db", "/tmp/x.sqlite"]));
        assert!(!daily_limit_alias_used(Vec::<String>::new()));
    }

    #[test]
    fn group_or_world_access_emits_warning_without_leaking_credentials() {
        let dir = TestDir::new("insecure-modes");

        for mode in [0o440, 0o640, 0o604] {
            let path = dir.path().join(format!("config-{mode:04o}.toml"));
            write_config(&path, mode);

            let (config, events) = load_with_captured_events(&path);
            assert_eq!(config.wigle.api_key, TEST_API_KEY);
            let output = rendered_events(&events);
            let expected_mode = format!("{mode:04o}");
            assert!(
                events.iter().any(|(level, message)| {
                    *level == tracing::Level::WARN
                        && message.contains("WiGLE credentials file")
                        && message.contains(&path.display().to_string())
                        && message.contains(&expected_mode)
                        && message.contains("another local user")
                        && message.contains("read or modify")
                        && message.contains("0600")
                }),
                "mode {expected_mode} must emit a clear permission warning; events:\n{output}"
            );
            assert!(
                !output.contains(TEST_API_KEY),
                "permission warning leaked the API key; events:\n{output}"
            );
        }
    }

    #[test]
    fn exact_systemd_config_credential_at_0440_does_not_warn() {
        let dir = TestDir::new("systemd-credential-exact");
        let credentials_directory = dir.path().join("run/credentials/whereamid.service");
        std::fs::create_dir_all(&credentials_directory)
            .expect("create systemd credential directory");
        let path = credentials_directory.join("config");
        write_config(&path, 0o440);
        let opened = File::open(&path).expect("open exact credential");
        let opened_metadata = opened.metadata().expect("stat opened exact credential");

        assert!(is_systemd_config_credential(
            &path,
            Some(&credentials_directory)
        ));
        assert!(direct_entry_identifies_open_file(&path, &opened_metadata));
        assert!(!is_systemd_config_credential(
            &path,
            Some(std::path::Path::new(""))
        ));
        assert!(!is_systemd_config_credential(
            &path,
            Some(std::path::Path::new("relative/credentials"))
        ));

        let (config, events) = load_with_captured_events_and_credentials_directory(
            &path,
            Some(&credentials_directory),
        );
        assert_eq!(config.wigle.api_key, TEST_API_KEY);
        let output = rendered_events(&events);
        assert!(
            !events.iter().any(|(level, message)| {
                *level == tracing::Level::WARN
                    && (message.contains("WiGLE credentials file")
                        || message.contains("another local user"))
            }),
            "systemd's exact protected 0440 config credential must not emit the generic \
             loose-file warning; events:\n{output}"
        );
    }

    #[test]
    fn exact_systemd_config_credential_symlink_at_0440_still_warns() {
        let dir = TestDir::new("systemd-credential-symlink");
        let credentials_directory = dir.path().join("run/credentials/whereamid.service");
        std::fs::create_dir_all(&credentials_directory)
            .expect("create systemd credential directory");
        let target = credentials_directory.join("target");
        let path = credentials_directory.join("config");
        write_config(&target, 0o440);
        std::os::unix::fs::symlink(&target, &path).expect("create exact-name credential symlink");

        let opened = File::open(&path).expect("open credential symlink target");
        let opened_metadata = opened.metadata().expect("stat opened symlink target");
        assert!(is_systemd_config_credential(
            &path,
            Some(&credentials_directory)
        ));
        assert!(std::fs::symlink_metadata(&path)
            .expect("lstat credential symlink")
            .file_type()
            .is_symlink());
        assert!(!direct_entry_identifies_open_file(&path, &opened_metadata));

        let (config, events) = load_with_captured_events_and_credentials_directory(
            &path,
            Some(&credentials_directory),
        );
        assert_eq!(config.wigle.api_key, TEST_API_KEY);
        let output = rendered_events(&events);
        assert!(
            events.iter().any(|(level, message)| {
                *level == tracing::Level::WARN
                    && message.contains("WiGLE credentials file")
                    && message.contains("0440")
                    && message.contains("another local user")
            }),
            "an exact-name 0440 symlink must not receive the managed-copy exemption; \
             events:\n{output}"
        );
    }

    #[test]
    fn exact_systemd_config_credential_unsafe_modes_still_warn() {
        let dir = TestDir::new("systemd-credential-unsafe-modes");

        for mode in [0o444, 0o640] {
            let credentials_directory = dir
                .path()
                .join(format!("run/credentials/whereamid-{mode:04o}.service"));
            std::fs::create_dir_all(&credentials_directory)
                .expect("create systemd credential directory");
            let path = credentials_directory.join("config");
            write_config(&path, mode);

            let (config, events) = load_with_captured_events_and_credentials_directory(
                &path,
                Some(&credentials_directory),
            );
            assert_eq!(config.wigle.api_key, TEST_API_KEY);
            let output = rendered_events(&events);
            let expected_mode = format!("{mode:04o}");
            assert!(
                events.iter().any(|(level, message)| {
                    *level == tracing::Level::WARN
                        && message.contains("WiGLE credentials file")
                        && message.contains(&expected_mode)
                        && message.contains("another local user")
                }),
                "exact managed path at future/unsafe mode {expected_mode} must warn; \
                 events:\n{output}"
            );
        }
    }

    #[test]
    fn systemd_credential_direct_entry_must_match_opened_descriptor() {
        let dir = TestDir::new("systemd-credential-path-identity");
        let path = dir.path().join("config");
        let original = dir.path().join("opened-original");
        write_config(&path, 0o440);

        let opened = File::open(&path).expect("open original credential");
        let opened_metadata = opened.metadata().expect("stat opened credential");
        assert!(direct_entry_identifies_open_file(&path, &opened_metadata));

        std::fs::rename(&path, &original).expect("move opened credential path");
        write_config(&path, 0o440);
        assert!(
            !direct_entry_identifies_open_file(&path, &opened_metadata),
            "a replacement path entry must not identify the already-open descriptor"
        );

        std::fs::remove_file(&path).expect("remove replacement credential");
        assert!(
            !direct_entry_identifies_open_file(&path, &opened_metadata),
            "direct-entry inspection failure must fail closed"
        );
    }

    #[test]
    fn systemd_credential_path_near_misses_at_0440_still_warn() {
        let dir = TestDir::new("systemd-credential-near-misses");
        let credentials_root = dir.path().join("run/credentials");
        let credentials_directory = credentials_root.join("whereamid.service");
        std::fs::create_dir_all(&credentials_directory)
            .expect("create systemd credential directory");

        let mut prefixed_directory = credentials_directory.as_os_str().to_os_string();
        prefixed_directory.push("-lookalike");
        let candidates = [
            (
                "near-miss name",
                credentials_directory.join("config.backup"),
            ),
            (
                "sibling directory",
                credentials_root.join("sibling.service/config"),
            ),
            (
                "directory-prefix lookalike",
                PathBuf::from(prefixed_directory).join("config"),
            ),
        ];

        for (case, path) in candidates {
            std::fs::create_dir_all(path.parent().expect("candidate has parent"))
                .expect("create candidate parent");
            write_config(&path, 0o440);
            assert!(
                !is_systemd_config_credential(&path, Some(&credentials_directory)),
                "{case} must not match the exact managed credential path"
            );

            let (config, events) = load_with_captured_events_and_credentials_directory(
                &path,
                Some(&credentials_directory),
            );
            assert_eq!(config.wigle.api_key, TEST_API_KEY);
            let output = rendered_events(&events);
            assert!(
                events.iter().any(|(level, message)| {
                    *level == tracing::Level::WARN
                        && message.contains("WiGLE credentials file")
                        && message.contains("0440")
                        && message.contains("another local user")
                }),
                "{case} at mode 0440 must retain the ordinary permission warning; \
                 events:\n{output}"
            );
        }
    }

    #[test]
    fn permission_warning_escapes_control_characters_in_config_path() {
        let dir = TestDir::new("control-path");
        let path = dir.path().join("config-\n-\u{1b}.toml");
        write_config(&path, 0o644);

        let (config, events) = load_with_captured_events(&path);
        assert_eq!(config.wigle.api_key, TEST_API_KEY);
        let output = rendered_events(&events);
        let message = events
            .iter()
            .find_map(|(level, message)| {
                (*level == tracing::Level::WARN && message.contains("WiGLE credentials file"))
                    .then_some(message.as_str())
            })
            .unwrap_or_else(|| panic!("missing permission warning; events:\n{output}"));
        let escaped_path = format!("{path:?}");

        assert!(
            message.contains(&escaped_path),
            "warning must preserve the configured path in escaped Debug form {escaped_path}; \
             events:\n{output}"
        );
        assert!(
            message.contains("\\n") && (message.contains("\\u{1b}") || message.contains("\\x1b")),
            "warning must visibly escape newline and ESC bytes; events:\n{output}"
        );
        assert!(
            !message.contains('\n') && !message.contains('\u{1b}'),
            "warning must not contain literal newline or ESC bytes; events:\n{output}"
        );
        assert!(
            message.contains("0644")
                && message.contains("another local user")
                && message.contains("read or modify")
                && message.contains("owner-only mode 0600"),
            "escaped-path warning must retain mode and risk guidance; events:\n{output}"
        );
        assert!(
            !message.contains(TEST_API_KEY),
            "escaped-path warning leaked the API key; events:\n{output}"
        );
    }

    #[test]
    fn owner_only_modes_do_not_emit_permission_warning() {
        let dir = TestDir::new("owner-only-modes");

        for mode in [0o600, 0o400] {
            let path = dir.path().join(format!("config-{mode:04o}.toml"));
            write_config(&path, mode);

            let (config, events) = load_with_captured_events(&path);
            assert_eq!(config.wigle.api_key, TEST_API_KEY);
            let output = rendered_events(&events);
            assert!(
                !events.iter().any(|(level, message)| {
                    *level == tracing::Level::WARN
                        && (message.contains("WiGLE credentials file")
                            || message.contains("cannot verify permissions"))
                }),
                "owner-only mode {mode:04o} must not emit a permission warning; events:\n{output}"
            );
        }
    }

    #[test]
    fn symlink_permission_check_uses_opened_target_mode() {
        let dir = TestDir::new("symlink-target-mode");
        let target = dir.path().join("target.toml");
        let link = dir.path().join("config.toml");
        write_config(&target, 0o600);
        std::os::unix::fs::symlink(&target, &link).expect("create config symlink");

        let (_, safe_events) = load_with_captured_events(&link);
        let safe_output = rendered_events(&safe_events);
        assert!(
            !safe_events.iter().any(|(level, message)| {
                *level == tracing::Level::WARN && message.contains("WiGLE credentials file")
            }),
            "symlink metadata must use its 0600 target, not the symlink's mode; events:\n{safe_output}"
        );

        let mut permissions = std::fs::metadata(&target)
            .expect("stat symlink target")
            .permissions();
        permissions.set_mode(0o644);
        std::fs::set_permissions(&target, permissions).expect("make symlink target insecure");

        let (_, insecure_events) = load_with_captured_events(&link);
        let insecure_output = rendered_events(&insecure_events);
        assert!(
            insecure_events.iter().any(|(level, message)| {
                *level == tracing::Level::WARN
                    && message.contains(&link.display().to_string())
                    && message.contains("0644")
            }),
            "symlink metadata must report its opened 0644 target and the configured path; \
             events:\n{insecure_output}"
        );
    }

    #[test]
    fn defaults_validate_clean() {
        let args = parse(&[]);
        args.validate().expect("default Args must validate");
        assert_eq!(args.history_retention_days, 30);
        assert_eq!(args.aps_retention_days, 30);
        assert_eq!(
            args.mobile_ssid_denylist,
            [
                "DSB_Wi-Fi",
                ".DSB_Wi-Fi",
                "dsb-mng-wifi",
                "MOVIA*",
                "Bus_WiFi",
                "Bus WiFi",
                "*_iPhone",
                "*_Hotspot",
                "iPhone",
                "AndroidAP*",
            ]
        );
    }

    #[test]
    fn mobile_ssid_denylist_parses_one_comma_separated_argument() {
        let args = parse(&["--mobile-ssid-denylist", "MOVIA*,Bus WiFi,?_Hotspot"]);
        assert_eq!(
            args.mobile_ssid_denylist,
            ["MOVIA*", "Bus WiFi", "?_Hotspot"]
        );
    }

    #[test]
    fn empty_mobile_ssid_denylist_disables_matching() {
        let args = parse(&["--mobile-ssid-denylist", ""]);
        assert!(!crate::scanner::is_mobile_ssid(
            Some("MOVIA_BUS_42"),
            &args.mobile_ssid_denylist
        ));
    }

    #[test]
    fn mobile_ssid_denylist_equals_form_keeps_leading_hyphen_as_data() {
        let args = parse(&["--mobile-ssid-denylist=--adversarial,MOVIA*"]);
        assert_eq!(args.mobile_ssid_denylist, ["--adversarial", "MOVIA*"]);
    }

    #[test]
    fn mobile_ssid_denylist_rejects_cr_and_lf() {
        for value in ["MOVIA*\nBus*", "MOVIA*\rBus*"] {
            let error = parse(&["--mobile-ssid-denylist", value])
                .validate()
                .expect_err("multiline pattern must fail validation");
            assert!(error.to_string().contains("--mobile-ssid-denylist"));
        }
    }

    #[test]
    fn retention_zero_is_accepted_and_out_of_range_is_rejected_by_clap() {
        parse(&["--history-retention-days", "0", "--aps-retention-days", "0"])
            .validate()
            .expect("zero explicitly disables retention");

        for flag in ["--history-retention-days", "--aps-retention-days"] {
            assert!(Args::try_parse_from(["whereamid", flag, "-1"]).is_err());
            assert!(Args::try_parse_from([
                "whereamid",
                flag,
                &(crate::db::MAX_RETENTION_DAYS + 1).to_string(),
            ])
            .is_err());
        }
    }

    #[test]
    fn max_connections_defaults_to_32() {
        let args = parse(&[]);
        assert_eq!(args.max_connections, 32);
        args.validate().unwrap();
    }

    #[test]
    fn max_connections_zero_is_rejected() {
        let error = parse(&["--max-connections", "0"])
            .validate()
            .expect_err("zero would make the daemon stop accepting forever");
        assert!(error.to_string().contains("--max-connections"));
    }

    #[test]
    fn max_connections_above_portable_limit_is_rejected_before_semaphore_new() {
        let error = parse(&["--max-connections", "4097"])
            .validate()
            .expect_err("portable policy must reject absurd connection caps");
        assert!(error.to_string().contains("--max-connections"));
        assert!(error.to_string().contains("4096"));
    }

    #[test]
    fn max_connections_portable_limit_is_accepted() {
        parse(&["--max-connections", "4096"])
            .validate()
            .expect("documented upper bound must validate");
    }

    #[test]
    fn debounce_threshold_greater_than_window_rejected() {
        let args = parse(&["--debounce-window", "3", "--debounce-threshold", "5"]);
        let err = args.validate().expect_err("expected validation error");
        let msg = format!("{err}");
        assert!(
            msg.contains("debounce-threshold"),
            "error message should name the offending flag, got: {msg}"
        );
    }

    #[test]
    fn debounce_threshold_equal_to_window_accepted() {
        parse(&["--debounce-window", "5", "--debounce-threshold", "5"])
            .validate()
            .expect("threshold == window must be allowed");
    }

    #[test]
    fn zero_window_rejected() {
        assert!(parse(&["--debounce-window", "0"]).validate().is_err());
    }

    #[test]
    fn zero_top_n_rejected() {
        assert!(parse(&["--top-n", "0"]).validate().is_err());
    }

    #[test]
    fn max_travel_speed_defaults_to_250() {
        let args = parse(&[]);
        assert_eq!(args.max_travel_speed_kmh, 250.0);
        args.validate().unwrap();
    }

    #[test]
    fn max_travel_speed_accepts_positive_fraction() {
        let args = parse(&["--max-travel-speed-kmh", "0.5"]);
        assert_eq!(args.max_travel_speed_kmh, 0.5);
        args.validate().unwrap();
    }

    #[test]
    fn max_travel_speed_rejects_non_positive_and_non_finite_values() {
        for argument in [
            "--max-travel-speed-kmh=0",
            "--max-travel-speed-kmh=-1",
            "--max-travel-speed-kmh=NaN",
            "--max-travel-speed-kmh=inf",
            "--max-travel-speed-kmh=-inf",
        ] {
            let error = parse(&[argument])
                .validate()
                .expect_err("invalid speed limit must fail validation");
            assert!(error.to_string().contains("--max-travel-speed-kmh"));
        }
    }

    #[test]
    fn zero_scan_interval_fast_rejected() {
        assert!(parse(&["--scan-interval-fast", "0"]).validate().is_err());
    }

    #[test]
    fn autolocate_interval_defaults_and_zero_off_switch() {
        let defaults = parse(&[]);
        assert_eq!(defaults.autolocate_interval_secs, 120);
        defaults.validate().unwrap();

        let disabled = parse(&["--autolocate-interval-secs", "0"]);
        assert_eq!(disabled.autolocate_interval_secs, 0);
        disabled.validate().expect("zero must disable autolocate");

        let custom = parse(&["--autolocate-interval-secs", "7"]);
        assert_eq!(custom.autolocate_interval_secs, 7);
        custom.validate().unwrap();
        let maximum = parse(&["--autolocate-interval-secs", "31536000"]);
        assert_eq!(
            maximum.autolocate_interval_secs,
            MAX_AUTOLOCATE_INTERVAL_SECS
        );
        maximum
            .validate()
            .expect("documented upper bound must validate");
        assert!(Args::try_parse_from(["whereamid", "--autolocate-interval-secs", "-1"]).is_err());
        assert!(
            Args::try_parse_from(["whereamid", "--autolocate-interval-secs", "31536001",]).is_err()
        );
        assert!(Args::try_parse_from([
            "whereamid",
            "--autolocate-interval-secs",
            "18446744073709551615",
        ])
        .is_err());

        let mut bypassing_clap = parse(&[]);
        bypassing_clap.autolocate_interval_secs = u64::MAX;
        let error = bypassing_clap
            .validate()
            .expect_err("validate must defensively reject direct invalid Args");
        assert!(error.to_string().contains("--autolocate-interval-secs"));
    }

    #[test]
    fn autolocate_help_names_default_and_off_switch() {
        use clap::CommandFactory as _;
        let mut help = Vec::new();
        Args::command().write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();
        let section = help
            .split("--autolocate-interval-secs")
            .nth(1)
            .expect("autolocate flag missing from help");
        assert!(section.contains("[default: 120]"));
        assert!(section.contains("0..=31536000"));
        assert!(section.contains("0 disables autonomous location history"));
    }

    #[test]
    fn autolocate_max_scan_age_defaults_zero_override_and_bounds() {
        let defaults = parse(&[]);
        assert_eq!(defaults.autolocate_max_scan_age_secs, 180);
        defaults.validate().unwrap();

        let legacy = parse(&["--autolocate-max-scan-age-secs", "0"]);
        assert_eq!(legacy.autolocate_max_scan_age_secs, 0);
        legacy
            .validate()
            .expect("zero must enable the legacy override");

        let maximum = parse(&["--autolocate-max-scan-age-secs", "31536000"]);
        assert_eq!(
            maximum.autolocate_max_scan_age_secs,
            MAX_AUTOLOCATE_SCAN_AGE_SECS
        );
        maximum
            .validate()
            .expect("documented maximum must validate");

        for invalid in ["-1", "31536001", "18446744073709551615"] {
            assert!(Args::try_parse_from(
                ["whereamid", "--autolocate-max-scan-age-secs", invalid,]
            )
            .is_err());
        }

        let mut bypassing_clap = parse(&[]);
        bypassing_clap.autolocate_max_scan_age_secs = u64::MAX;
        let error = bypassing_clap
            .validate()
            .expect_err("validate must reject a directly-constructed invalid value");
        assert!(error.to_string().contains("--autolocate-max-scan-age-secs"));
    }

    #[test]
    fn autolocate_max_scan_age_help_pins_default_boundary_and_zero_semantics() {
        use clap::CommandFactory as _;
        let mut help = Vec::new();
        Args::command().write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();
        let section = help
            .split("--autolocate-max-scan-age-secs")
            .nth(1)
            .expect("scan-age flag missing from help")
            .split("--debounce-window")
            .next()
            .unwrap();
        assert!(section.contains("[default: 180]"));
        assert!(section.contains("0..=31536000"));
        assert!(section.contains("Equality is fresh"));
        assert!(section.contains("0 accepts any present snapshot"));
    }

    // --- task-0196: --bind opt-in TCP + non-loopback guard ---

    #[test]
    fn no_bind_is_valid_and_tcp_disabled() {
        // TCP is opt-in: the default (no --bind) must validate and report no
        // TCP address.
        let args = parse(&[]);
        args.validate().expect("no --bind must validate");
        assert_eq!(args.parsed_bind().unwrap(), None);
    }

    #[test]
    fn bind_garbage_rejected() {
        // The old raw-String field accepted any junk; now a malformed bind
        // fails fast at validation time.
        let err = parse(&["--bind", "not-an-address"])
            .validate()
            .expect_err("garbage --bind must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("--bind"),
            "error must name the offending flag, got: {msg}"
        );
    }

    #[test]
    fn bind_loopback_accepted_without_flag() {
        parse(&["--bind", "127.0.0.1:4747"])
            .validate()
            .expect("loopback --bind must be allowed without the opt-in flag");
        parse(&["--bind", "[::1]:4747"])
            .validate()
            .expect("IPv6 loopback --bind must be allowed without the opt-in flag");
    }

    #[test]
    fn bind_nonloopback_without_flag_rejected_names_flag() {
        let err = parse(&["--bind", "0.0.0.0:4747"])
            .validate()
            .expect_err("non-loopback --bind must require the opt-in flag");
        let msg = format!("{err}");
        assert!(
            msg.contains("--allow-nonlocal-bind"),
            "error must name the opt-in flag so the user knows how to proceed, got: {msg}"
        );
    }

    #[test]
    fn bind_nonloopback_with_flag_accepted() {
        parse(&["--bind", "0.0.0.0:4747", "--allow-nonlocal-bind"])
            .validate()
            .expect("non-loopback --bind must be allowed once explicitly opted in");
    }

    #[test]
    fn parsed_bind_is_single_source_of_truth() {
        // The parsed address run_server will bind must match exactly what
        // validate() inspected.
        let args = parse(&["--bind", "127.0.0.1:4747"]);
        let addr = args.parsed_bind().unwrap().unwrap();
        assert_eq!(addr.port(), 4747);
        assert!(addr.ip().is_loopback());
    }

    #[test]
    fn allow_nonlocal_bind_without_bind_rejected() {
        // The flag alone would be a silent no-op — the operator who passes it
        // believes TCP is on. Fail fast instead (task-0196 review, mped L3).
        let err = parse(&["--allow-nonlocal-bind"])
            .validate()
            .expect_err("--allow-nonlocal-bind without --bind must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("--bind"),
            "error must point at the missing flag, got: {msg}"
        );
    }

    #[test]
    fn empty_socket_path_rejected() {
        // An empty --socket would otherwise fail later at bind time with a
        // confusing error instead of at validation (task-0196 review, mped L4).
        assert!(parse(&["--socket", ""]).validate().is_err());
        assert!(parse(&["--socket", "  "]).validate().is_err());
    }
}
