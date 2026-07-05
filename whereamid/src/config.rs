//! CLI arg parsing (clap) and TOML config file loading for secrets.

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// Portable operational ceiling for active control-channel connections.
/// Deliberately far below Tokio's target-dependent semaphore limit on every
/// supported target, while still allowing unusually busy local deployments.
pub const MAX_CONNECTIONS: usize = 4096;
const _: () = assert!(MAX_CONNECTIONS <= tokio::sync::Semaphore::MAX_PERMITS);

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

    /// Path to TOML WiGLE API credentials file; keep owner-only (0600)
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

    /// Number of scan samples in debounce ring buffer
    #[arg(long, default_value_t = 10)]
    pub debounce_window: usize,

    /// Minimum appearances in ring buffer to be considered stable
    #[arg(long, default_value_t = 5)]
    pub debounce_threshold: usize,

    /// Only consider top-N strongest APs for trilateration
    #[arg(long, default_value_t = 10)]
    pub top_n: usize,

    /// Seconds between pending queue drain runs
    #[arg(long, default_value_t = 300)]
    pub pending_interval: u64,

    /// Drop from pending after this many failed attempts
    #[arg(long, default_value_t = 20)]
    pub pending_max_attempts: i32,

    /// Maximum WiGLE API calls per day
    #[arg(long, default_value_t = 100)]
    pub daily_limit: u32,

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
    #[arg(long, default_value_t = 30)]
    pub history_retention_days: i64,

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
        if self.top_n == 0 {
            anyhow::bail!("--top-n must be > 0");
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
        if self.history_retention_days < 0 {
            anyhow::bail!("--history-retention-days must be >= 0 (0 disables retention)");
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

/// Load the TOML config file. Returns default if file does not exist.
pub fn load_config_file(path: &str) -> Result<ConfigFile> {
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
            if mode & 0o077 != 0 {
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
    use std::sync::{Arc, Mutex};
    use tracing::{
        field::{Field, Visit},
        span, subscriber, Event, Metadata, Subscriber,
    };

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

    #[derive(Clone, Default)]
    struct CapturedEvents(Arc<Mutex<Vec<(tracing::Level, String)>>>);

    struct MessageVisitor(String);

    impl Visit for MessageVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.0 = format!("{value:?}");
            }
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "message" {
                self.0 = value.to_owned();
            }
        }
    }

    impl Subscriber for CapturedEvents {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _attributes: &span::Attributes<'_>) -> span::Id {
            span::Id::from_u64(1)
        }

        fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}
        fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut visitor = MessageVisitor(String::new());
            event.record(&mut visitor);
            self.0
                .lock()
                .expect("capture lock poisoned")
                .push((*event.metadata().level(), visitor.0));
        }

        fn enter(&self, _span: &span::Id) {}
        fn exit(&self, _span: &span::Id) {}
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
        let captured = CapturedEvents::default();
        let events = captured.clone();
        let config = subscriber::with_default(captured, || {
            load_config_file(path.to_str().expect("UTF-8 config test path"))
        })
        .expect("load config test file");
        let events = events.0.lock().expect("capture lock poisoned").clone();
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
    fn group_or_world_access_emits_warning_without_leaking_credentials() {
        let dir = TestDir::new("insecure-modes");

        for mode in [0o640, 0o604] {
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
        parse(&[]).validate().expect("default Args must validate");
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
    fn zero_scan_interval_fast_rejected() {
        assert!(parse(&["--scan-interval-fast", "0"]).validate().is_err());
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
