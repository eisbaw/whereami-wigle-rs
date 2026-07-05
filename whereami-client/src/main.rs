//! `whereami` CLI: typed front-end for the whereamid daemon.
//!
//! task-0079 moved this binary off ad-hoc argv scraping onto clap's
//! derive API. The previous shape silently accepted `--json` and
//! `--scan-time=no` anywhere on the command line and didn't list them
//! in `whereami help`; the per-subcommand `whereami history --from/--to`
//! parsing was hand-rolled. clap now drives parsing for every flag,
//! `--help` is auto-generated, and unknown flags fail fast.
//!
//! Backward compat:
//! - `whereami` with no subcommand still defaults to `locate`.
//! - `whereami --json scan` still works (global `--json`).
//! - `whereami scan --scan-time=no` still works (the value-taking
//!   `--scan-time <yes|no>` flag lives on the scan subcommand).
//! - Single-letter aliases preserved (l/s/st/d/h/v).
//! - `--version` / `-V` print the embedded CLI version locally, without
//!   resolving or contacting a daemon. The `version` subcommand continues
//!   to query the daemon and report both versions.

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use std::{process, time::Duration};
use whereami_client::{
    DaemonResponse, LocateResponse, ScanResponse, StatsResponse, WhereAmIClient, WipeConfirmation,
    WipeResponse, DEFAULT_SOCKET_IO_TIMEOUT,
};

/// `whereami` — Wi-Fi geolocation CLI.
///
/// Talks JSON-lines to the local whereamid daemon over its Unix domain socket
/// (default `$XDG_RUNTIME_DIR/whereamid.sock`, else `/run/whereami/whereamid.sock`).
/// Override with `WHEREAMI_SOCKET=<path>`; point at an old TCP daemon with
/// `WHEREAMI_ADDR=<host:port>`. Output is human-readable by default; pass
/// `--json` for JSON output; daemon responses pass through unchanged.
#[derive(Parser, Debug)]
#[command(
    name = "whereami",
    bin_name = "whereami",
    about = "Wi-Fi geolocation CLI (talks to whereamid)",
    long_about = None,
    // We provide our own --version so it can include the embedded Git
    // revision and support the CLI's global --json output mode.
    disable_version_flag = true,
    arg_required_else_help = false,
    // task-0199: document the Unix-filter pipe behavior in `whereami --help`.
    after_help = "Pipes: whereami behaves like a standard Unix filter. If a downstream \
                  reader closes the pipe early (e.g. `whereami scan | head`), the process \
                  is terminated by SIGPIPE (exit status 141) rather than printing a panic.",
)]
struct Cli {
    /// Print JSON instead of human-readable output; daemon responses stay raw.
    #[arg(long, global = true)]
    json: bool,

    /// Socket read/write timeout in seconds; use 0 to disable it for long
    /// batch or history operations.
    #[arg(
        long,
        global = true,
        value_name = "SECONDS",
        default_value_t = DEFAULT_SOCKET_IO_TIMEOUT.as_secs()
    )]
    timeout_secs: u64,

    /// Print this CLI's version locally, without contacting the daemon.
    #[arg(long = "version", short = 'V', global = true)]
    version_flag: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

/// Subcommands. Single-letter aliases match the prior CLI: locate->l,
/// scan->s, stats->st, debug->d, history->h, version->v.
#[derive(Subcommand, Debug)]
enum Command {
    /// Where am I? (default if no subcommand is given)
    #[command(alias = "l")]
    Locate,

    /// List visible Wi-Fi networks from the most recent scan.
    #[command(alias = "s")]
    Scan {
        /// Suppress the "scanned: TIMESTAMP (Ns ago)" header line. Defaults
        /// to `yes` (i.e. show the header). The legacy invocation
        /// `whereami scan --scan-time=no` still works.
        #[arg(long, value_enum, default_value_t = YesNo::Yes)]
        scan_time: YesNo,
    },

    /// Cache/rate counters and tasks aborted at the last clean shutdown.
    #[command(alias = "st")]
    Stats,

    /// Per-BSSID debugger snapshot: scan buffer state and DB classification.
    #[command(alias = "d")]
    Debug,

    /// Stay-point segments from the location-history timeseries.
    ///
    /// Defaults to the last 7 days. Pass either a relative range
    /// (`24h`, `7d`, `1w`) OR both `--from` and `--to` as RFC3339
    /// timestamps; the two forms are mutually exclusive.
    #[command(alias = "h")]
    History {
        /// Relative range: `24h`, `7d`, `1w`. Mutually exclusive with
        /// --from/--to.
        range: Option<String>,
        /// RFC3339 inclusive start. Requires --to.
        #[arg(long, conflicts_with = "range")]
        from: Option<String>,
        /// RFC3339 inclusive end. Requires --from.
        #[arg(long, conflicts_with = "range")]
        to: Option<String>,
    },

    /// Delete all stored location, history, and observed Wi-Fi data.
    Wipe {
        /// Required non-interactive confirmation of the destructive operation.
        #[arg(long, required = true)]
        yes: bool,
    },

    /// Contact the daemon and print both CLI and daemon versions.
    #[command(alias = "v")]
    Version,
}

/// `--scan-time <yes|no>` enum. Modeled this way (instead of a bare
/// `--no-scan-time` flag) to preserve the legacy invocation
/// `whereami scan --scan-time=no` byte-for-byte.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum YesNo {
    Yes,
    No,
}

/// Print a fatal error and exit with code 1.
fn fatal(msg: &str) -> ! {
    eprintln!("{msg}");
    process::exit(1);
}

/// Human-mode dispatch ladder: ok=false fatals with the daemon's error,
/// ok=true delegates to render, transport error fatals. The `--json` path
/// is now handled separately by `emit_json` (task-0183), so this no longer
/// branches on json and never re-serializes the parsed struct.
fn dispatch<T: DaemonResponse>(result: Result<T>, render: impl FnOnce(&T)) {
    match result {
        // Sanitize the daemon-sourced error too: it can echo attacker
        // input and is printed straight to the terminal (task-0198).
        Ok(resp) if !resp.is_ok() => fatal(&sanitize(resp.error().unwrap_or("unknown error"))),
        Ok(resp) => render(&resp),
        // Transport errors are CLI/OS-origin today, but sanitize anyway in
        // case a future error variant embeds daemon-controlled bytes.
        Err(e) => fatal(&sanitize(&format!("error: {e}"))),
    }
}

/// Build a CLI-synthesized error for a daemon-backed `--json` command. Used
/// when there is no daemon line to pass through (daemon unreachable, or it
/// returned an unusable/non-JSON payload). Serialized via serde_json so the
/// message is always correctly escaped and stdout stays valid JSON (task-0183
/// AC #3).
///
/// NOTE: this object is fabricated by the CLI, not received from the
/// daemon. The separate offline `--json --version` flag also fabricates a
/// local object, but it is a successful client-version result rather than an
/// error. protocol.md documents both CLI-owned shapes (see TASK-0213 and
/// TASK-0236).
fn synth_error(msg: &str) -> String {
    serde_json::json!({ "ok": false, "error": msg }).to_string()
}

/// Pure core of the daemon-backed `--json` path (task-0183): map the raw-line
/// result to the exact stdout line and process exit code. No I/O, so it is
/// directly unit-testable.
///
/// - Valid JSON line: passed through BYTE-IDENTICALLY. Exit 0 iff its `ok`
///   field is boolean true, else exit 1. We parse ONLY `ok` and never
///   re-serialize, so no field absent on the wire is invented and no
///   present/unknown field is dropped.
/// - Non-JSON line (broken daemon): treated as a transport failure — a
///   synthesized error embedding the offending payload, exit 1.
/// - Transport error (daemon down): synthesized error, exit 1.
fn json_output(raw: Result<String>) -> (String, i32) {
    match raw {
        Ok(line) => match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(value) => {
                let ok = value.get("ok").and_then(serde_json::Value::as_bool) == Some(true);
                (line, i32::from(!ok))
            }
            Err(_) => (
                synth_error(&format!("daemon returned a non-JSON response: {line}")),
                1,
            ),
        },
        Err(e) => (synth_error(&format!("{e:#}")), 1),
    }
}

/// Emit the `--json` result and exit. Never returns.
///
/// Daemon-backed callers must pair this with the human path in an
/// `if json { emit_json(..) } else { dispatch(..) }` so the two modes are
/// mutually exclusive by control flow, not merely by this `-> !`
/// signature — otherwise a future non-diverging refactor would silently
/// fall through into a second daemon call and human output after the JSON
/// line was already printed.
fn emit_json(raw: Result<String>) -> ! {
    let (line, code) = json_output(raw);
    println!("{line}");
    process::exit(code);
}

/// Escape control characters in daemon-sourced strings before printing
/// them to a terminal, so an attacker-controlled SSID or address cannot
/// smuggle ANSI/OSC escape sequences into the operator's terminal
/// (task-0198). `char::is_control()` covers C0 (U+0000..=U+001F), DEL
/// (U+007F) and C1 (U+0080..=U+009F) — including ESC 0x1B, CR 0x0D,
/// BEL 0x07 and CSI 0x9B — which become visible `\u{..}`/`\r`-style
/// escapes via `char::escape_default`. Every other char (printable ASCII,
/// emoji, non-Latin scripts) passes through unchanged so legitimate SSIDs
/// still render readably (AC #3).
///
/// Applied ONLY in the human renderer. Daemon lines on the daemon-backed
/// `--json` path pass through byte-identically (task-0183) and must stay
/// untouched — that IS task-0198 AC #2.
fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

/// Render a `scan` response to the human-readable multi-line string
/// (trailing newline per line). Daemon-sourced strings — SSID, BSSID and
/// the scan timestamp — are sanitized against terminal-escape injection
/// (task-0198). Pure, so it is directly unit-testable (AC #4).
fn render_scan(resp: &ScanResponse, no_scan_time: bool) -> String {
    let mut out = String::new();
    if !no_scan_time {
        let age_ms = resp.scan_age_ms.unwrap_or(0);
        let age_s = age_ms / 1000;
        let scanned_at = resp.scanned_at.as_deref().unwrap_or("?");
        out.push_str(&format!(
            "scanned: {}  ({age_s}s ago)\n",
            sanitize(scanned_at)
        ));
        out.push('\n');
    }
    for net in &resp.networks {
        let channel = net
            .channel
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".into());
        let ssid = net
            .ssid
            .as_deref()
            .map(sanitize)
            .unwrap_or_else(|| "<hidden>".into());
        out.push_str(&format!(
            "{:17}  {:>4} dBm  ch{:<3}  {}\n",
            sanitize(&net.bssid),
            net.signal_dbm,
            channel,
            ssid
        ));
    }
    out
}

/// Render a `locate` response's stdout body (optional address line plus
/// the coordinate line). The daemon-sourced address is sanitized against
/// terminal-escape injection (task-0198); the stale banner is written to
/// stderr by the caller. Pure, so it is directly unit-testable.
fn render_locate(resp: &LocateResponse) -> String {
    let mut out = String::new();
    if let Some(addr) = resp.address.as_deref() {
        if !addr.is_empty() {
            out.push_str(&sanitize(addr));
            out.push('\n');
        }
    }
    out.push_str(&format!(
        "{:.6}, {:.6}  ±{}m  ({} sources)\n",
        resp.lat, resp.lon, resp.accuracy_m as i64, resp.sources
    ));
    out
}

/// Render the stats response exactly as the human CLI prints it. The final
/// counter is explicitly labelled as belonging to the last clean shutdown:
/// the daemon persists it while shutting down and reloads it on its next start.
fn render_stats(resp: &StatsResponse) -> String {
    format!(
        "cached:        {}\n\
         pending:       {}\n\
         not_found:     {}\n\
         inflight:      {}\n\
         db size:       {} KB\n\
         API today:     {}\n\
         API counter corruption denials: {}\n\
         db write fail: {}\n\
         tasks aborted at last clean shutdown: {}\n",
        resp.cached_aps,
        resp.pending_aps,
        resp.not_found_aps,
        resp.inflight,
        resp.db_size_bytes / 1024,
        resp.api_calls_today,
        resp.api_counter_corruption_denials,
        resp.db_write_failures,
        resp.orphan_tasks_aborted,
    )
}

fn render_wipe(resp: &WipeResponse) -> String {
    format!(
        "deleted: aps={} pending={} not_found={} fixes={} last_fix={}\n\
         collection remains enabled; new observations may repopulate the database\n",
        resp.aps_deleted,
        resp.pending_deleted,
        resp.not_found_deleted,
        resp.fixes_deleted,
        resp.last_fix_deleted,
    )
}

/// Apply the CLI's seconds-based policy to the library client. Zero is an
/// explicit opt-out; every positive value uses the validating builder.
fn configure_client_timeout(client: WhereAmIClient, timeout_secs: u64) -> Result<WhereAmIClient> {
    if timeout_secs == 0 {
        Ok(client.without_io_timeout())
    } else {
        client.with_io_timeout(Duration::from_secs(timeout_secs))
    }
}

fn main() {
    // Restore the default SIGPIPE disposition (task-0199). The Rust runtime
    // installs SIG_IGN for SIGPIPE before main(), which turns a closed
    // downstream pipe (e.g. `whereami scan | head`) into an EPIPE that
    // println! escalates into a panic ("failed printing to stdout: Broken
    // pipe") and exit code 101. Resetting to SIG_DFL makes the process die
    // quietly from SIGPIPE (exit status 141), matching cat/grep and every
    // other well-behaved Unix filter.
    //
    // The disposition is process-wide, so it also covers writes to the
    // daemon socket (unix or opt-in TCP; lib.rs raw_command): a daemon that
    // accepts the connection and then closes mid-write kills the process with
    // 141 instead of the clean synthesized-JSON transport error. Still a
    // loud, nonzero failure — an accepted trade-off for pipe correctness.
    //
    // Safety: signal() is async-signal-safe; we call it exactly once, before
    // spawning threads or doing any I/O, with a constant handler — nothing
    // can race with it.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    // Use try_parse so we can normalise the exit code. clap's default
    // is 2 for parse errors; the prior CLI exited 1 for unknown
    // subcommands. Preserving exit code 1 keeps shell wrappers (e.g.
    // `whereami stats || journal -u whereamid`) behaving identically.
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            // Help/version messages still print to stdout with exit 0;
            // genuine errors print to stderr with exit 1.
            let kind = e.kind();
            // clap renders the message; we just override the exit code.
            let _ = e.print();
            let code = match kind {
                clap::error::ErrorKind::DisplayHelp
                | clap::error::ErrorKind::DisplayVersion
                | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => 0,
                _ => 1,
            };
            process::exit(code);
        }
    };

    // The conventional version flags describe this executable and must work
    // when the daemon is absent or its environment is misconfigured. Keep
    // this before daemon target resolution and timeout configuration.
    if cli.version_flag {
        print_local_version(cli.json);
        return;
    }

    let json = cli.json;
    let timeout_secs = cli.timeout_secs;
    // Resolve the daemon target (unix socket by default; TCP via WHEREAMI_ADDR).
    // A resolution failure (no socket found and no override set) is routed
    // through the SAME json/human split as a transport error, so `--json` still
    // emits a single synthesized JSON line to stdout (task-0183) rather than
    // only a stderr message — and the sigpipe/pipe semantics stay identical.
    let client = match WhereAmIClient::from_env()
        .and_then(|client| configure_client_timeout(client, timeout_secs))
    {
        Ok(c) => c,
        Err(e) => {
            if json {
                emit_json(Err(e));
            } else {
                fatal(&sanitize(&format!("error: {e}")));
            }
        }
    };

    // No subcommand -> default to locate (backwards compatible with the
    // pre-clap CLI that did `cmd = args.get(1).unwrap_or("locate")`).
    let command = cli.command.unwrap_or(Command::Locate);

    match command {
        Command::Locate => run_locate(&client, json),
        Command::Scan { scan_time } => run_scan(&client, json, scan_time == YesNo::No),
        Command::Stats => run_stats(&client, json),
        Command::Debug => run_debug(&client, json),
        Command::History { range, from, to } => run_history(&client, json, range, from, to),
        Command::Wipe { yes: true } => run_wipe(&client, json),
        Command::Wipe { yes: false } => unreachable!("clap requires --yes"),
        Command::Version => run_daemon_version(&client, json),
    }
}

fn run_locate(client: &WhereAmIClient, json: bool) {
    if json {
        emit_json(client.locate_raw());
    } else {
        dispatch(client.locate(), |resp| {
            if resp.stale {
                let age_s = resp.age_s.unwrap_or(0);
                let age_str = if age_s < 60 {
                    format!("{age_s}s")
                } else if age_s < 3600 {
                    format!("{}m", age_s / 60)
                } else {
                    format!("{}h", age_s / 3600)
                };
                eprint!("[last known, {age_str} ago] ");
            }
            print!("{}", render_locate(resp));
        });
    }
}

fn run_scan(client: &WhereAmIClient, json: bool, no_scan_time: bool) {
    if json {
        emit_json(client.scan_raw());
    } else {
        dispatch(client.scan(), |resp| {
            print!("{}", render_scan(resp, no_scan_time));
        });
    }
}

fn run_stats(client: &WhereAmIClient, json: bool) {
    if json {
        emit_json(client.stats_raw());
    } else {
        dispatch(client.stats(), |resp| {
            print!("{}", render_stats(resp));
        });
    }
}

fn run_debug(client: &WhereAmIClient, json: bool) {
    if json {
        emit_json(client.debug_raw());
    } else {
        dispatch(client.debug(), |resp| {
            let cli_rev = env!("GIT_REV");
            let daemon_rev = resp.daemon_rev.as_deref().unwrap_or("?");
            // daemon_rev is daemon-sourced; sanitize before printing (task-0198).
            println!("cli: {cli_rev}  daemon: {}", sanitize(daemon_rev));
            println!();
            let age = resp.scan_age_ms.unwrap_or(0);
            println!(
                "scan age: {}ms  samples: {}  visible: {}  stable: {}",
                age, resp.samples_in_buffer, resp.visible, resp.stable
            );
            println!();
            for b in &resp.bssids {
                let marker = if b.is_stable { "*" } else { " " };
                let signal = match b.signal_dbm {
                    Some(s) => format!("{s:>4} dBm"),
                    // task-0051: '-- dBm' marks stable BSSIDs absent
                    // from the latest scan (no current signal).
                    None => "  -- dBm".to_string(),
                };
                println!(
                    "{}{:17}  {}  {}/{}  {}",
                    marker,
                    sanitize(&b.bssid),
                    signal,
                    b.seen,
                    b.needed,
                    b.db_status,
                );
            }
            println!();
            println!("* = stable  seen/needed = debounce count/threshold");
        });
    }
}

fn run_history(
    client: &WhereAmIClient,
    json: bool,
    range: Option<String>,
    from: Option<String>,
    to: Option<String>,
) {
    // clap enforces --from/--to are mutually exclusive with `range`.
    // We additionally check that --from and --to come as a pair: passing
    // only one would let the daemon's history handler error, but we get
    // a better message here.
    match (&from, &to) {
        (Some(_), None) => fatal("history: --from requires --to"),
        (None, Some(_)) => fatal("history: --to requires --from"),
        _ => {}
    }
    // Default range = 7d if no explicit range/from-to.
    let range = if range.is_none() && from.is_none() && to.is_none() {
        Some("7d".to_string())
    } else {
        range
    };
    // Argument validation above (usage errors) is reported on stderr in
    // both modes, matching clap; from here on the daemon response — or a
    // synthesized transport error — is what reaches stdout under --json.
    // Whether these CLI-side usage errors should instead emit synthesized
    // JSON under --json is tracked in TASK-0228 (deliberate for now).
    if json {
        emit_json(client.history_raw(range, from, to));
    } else {
        dispatch(client.history(range, from, to), |resp| {
            // from/to and the per-segment timestamps are daemon-sourced;
            // sanitize before printing (task-0198).
            println!("range: {} -> {}", sanitize(&resp.from), sanitize(&resp.to));
            println!("segments: {}", resp.segments.len());
            for seg in &resp.segments {
                let dur_min = seg.duration_secs / 60;
                println!(
                    "  {} -> {}  {:.6},{:.6}  ±{:.0}m  {} fixes  ({}m)",
                    sanitize(&seg.start_rfc3339),
                    sanitize(&seg.end_rfc3339),
                    seg.centroid_lat,
                    seg.centroid_lon,
                    seg.mean_accuracy_m,
                    seg.n_fixes,
                    dur_min
                );
            }
        });
    }
}

fn run_wipe(client: &WhereAmIClient, json: bool) {
    if json {
        emit_json(client.wipe_raw(WipeConfirmation::Confirmed));
    } else {
        dispatch(client.wipe(WipeConfirmation::Confirmed), |resp| {
            print!("{}", render_wipe(resp));
        });
    }
}

/// Print the version embedded in this executable without resolving or
/// contacting a daemon. The JSON object is synthesized locally and serialized
/// by serde_json so future revision strings cannot corrupt the output shape.
fn print_local_version(json: bool) {
    let cli_version = env!("CARGO_PKG_VERSION");
    let cli_rev = env!("GIT_REV");
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "version": cli_version,
                "git_rev": cli_rev,
            })
        );
    } else {
        println!("cli:    {cli_version} ({cli_rev})");
    }
}

fn run_daemon_version(client: &WhereAmIClient, json: bool) {
    // --json version must never print the human `cli: ...` banner to
    // stdout; it goes through the same raw passthrough as every other
    // command, so stdout stays valid JSON even with the daemon down
    // (task-0183 AC #3).
    if json {
        emit_json(client.version_raw());
    } else {
        let cli_version = env!("CARGO_PKG_VERSION");
        let cli_rev = env!("GIT_REV");
        match client.version() {
            Ok(resp) if !resp.ok => {
                println!("cli:    {cli_version} ({cli_rev})");
                // Daemon-sourced error string; sanitize before printing (task-0198).
                eprintln!(
                    "daemon: {}",
                    sanitize(resp.error.as_deref().unwrap_or("unknown error"))
                );
                process::exit(1);
            }
            Ok(resp) => {
                let daemon_version = resp.version.as_deref().unwrap_or("?");
                let daemon_rev = resp.git_rev.as_deref().unwrap_or("?");
                println!("cli:    {cli_version} ({cli_rev})");
                // Daemon-sourced strings; sanitize before printing (task-0198).
                println!(
                    "daemon: {} ({})",
                    sanitize(daemon_version),
                    sanitize(daemon_rev)
                );
            }
            Err(e) => {
                println!("cli:    {cli_version} ({cli_rev})");
                // CLI/OS-origin today; sanitized for the same reason as
                // dispatch's transport-error arm.
                eprintln!("daemon: error: {}", sanitize(&e.to_string()));
                process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! task-0079: smoke tests for the clap surface. These pin the CLI
    //! shape so a future refactor can't accidentally drop a subcommand
    //! or rename an alias.
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_compiles() {
        // clap performs structural checks here (conflicting args,
        // duplicate names, etc.). If the derive ever drifts we want
        // a unit-test failure, not a runtime panic.
        Cli::command().debug_assert();
    }

    #[test]
    fn empty_args_is_valid_and_routes_to_default_subcommand() {
        // `whereami` with no subcommand must parse and produce
        // `command: None`; main() turns that into Locate.
        let cli = Cli::try_parse_from(["whereami"]).unwrap();
        assert!(cli.command.is_none());
        assert!(!cli.json);
        assert!(!cli.version_flag);
        assert_eq!(
            cli.timeout_secs,
            DEFAULT_SOCKET_IO_TIMEOUT.as_secs(),
            "CLI default must stay aligned with the library default"
        );
    }

    #[test]
    fn timeout_option_parses_and_wires_custom_and_disabled_values() {
        let custom = Cli::try_parse_from(["whereami", "history", "--timeout-secs", "120"]).unwrap();
        assert_eq!(custom.timeout_secs, 120);
        let custom_client =
            configure_client_timeout(WhereAmIClient::tcp("127.0.0.1:1"), custom.timeout_secs)
                .unwrap();
        assert_eq!(custom_client.io_timeout(), Some(Duration::from_secs(120)));

        let disabled = Cli::try_parse_from(["whereami", "--timeout-secs", "0"]).unwrap();
        assert_eq!(disabled.timeout_secs, 0);
        let disabled_client =
            configure_client_timeout(WhereAmIClient::tcp("127.0.0.1:1"), disabled.timeout_secs)
                .unwrap();
        assert_eq!(disabled_client.io_timeout(), None);
    }

    #[test]
    fn timeout_help_documents_disable_and_long_operations() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("--timeout-secs <SECONDS>"), "{help}");
        assert!(help.contains("0 to disable"), "{help}");
        assert!(help.contains("batch or history operations"), "{help}");
        assert!(help.contains("[default: 40]"), "{help}");
    }

    #[test]
    fn json_flag_can_appear_before_subcommand() {
        // `whereami --json scan` was the legacy invocation; clap's
        // global flag must accept that.
        let cli = Cli::try_parse_from(["whereami", "--json", "scan"]).unwrap();
        assert!(cli.json);
        assert!(matches!(cli.command, Some(Command::Scan { .. })));
    }

    #[test]
    fn legacy_scan_time_no_invocation_still_parses() {
        // Backwards compat: `whereami scan --scan-time=no` is documented
        // in old user notes. Must continue to resolve to no_scan_time=true.
        let cli = Cli::try_parse_from(["whereami", "scan", "--scan-time=no"]).unwrap();
        match cli.command {
            Some(Command::Scan { scan_time }) => assert_eq!(scan_time, YesNo::No),
            other => panic!("expected Scan, got {other:?}"),
        }
    }

    #[test]
    fn history_range_positional_parses() {
        let cli = Cli::try_parse_from(["whereami", "history", "24h"]).unwrap();
        match cli.command {
            Some(Command::History { range, from, to }) => {
                assert_eq!(range.as_deref(), Some("24h"));
                assert!(from.is_none());
                assert!(to.is_none());
            }
            other => panic!("expected History, got {other:?}"),
        }
    }

    #[test]
    fn history_from_to_parses() {
        let cli = Cli::try_parse_from([
            "whereami",
            "history",
            "--from",
            "2026-01-01T00:00:00Z",
            "--to",
            "2026-01-02T00:00:00Z",
        ])
        .unwrap();
        match cli.command {
            Some(Command::History { range, from, to }) => {
                assert!(range.is_none());
                assert_eq!(from.as_deref(), Some("2026-01-01T00:00:00Z"));
                assert_eq!(to.as_deref(), Some("2026-01-02T00:00:00Z"));
            }
            other => panic!("expected History, got {other:?}"),
        }
    }

    #[test]
    fn history_range_conflicts_with_from_to() {
        // clap should reject the combination at parse time.
        let res = Cli::try_parse_from([
            "whereami",
            "history",
            "7d",
            "--from",
            "2026-01-01T00:00:00Z",
        ]);
        assert!(res.is_err(), "range + --from must conflict");
    }

    #[test]
    fn wipe_requires_explicit_yes_and_parses_under_json() {
        assert!(
            Cli::try_parse_from(["whereami", "wipe"]).is_err(),
            "bare wipe must fail before target resolution"
        );
        let cli = Cli::try_parse_from(["whereami", "--json", "wipe", "--yes"]).unwrap();
        assert!(cli.json);
        assert!(matches!(cli.command, Some(Command::Wipe { yes: true })));
    }

    #[test]
    fn wipe_renderer_prints_exact_counts_and_repopulation_notice() {
        let output = render_wipe(&WipeResponse {
            ok: true,
            v: 1,
            aps_deleted: 5,
            pending_deleted: 4,
            not_found_deleted: 3,
            fixes_deleted: 2,
            last_fix_deleted: 1,
            error: None,
        });
        assert_eq!(
            output,
            "deleted: aps=5 pending=4 not_found=3 fixes=2 last_fix=1\n\
             collection remains enabled; new observations may repopulate the database\n"
        );
    }

    #[test]
    fn aliases_resolve_to_canonical_subcommands() {
        // Single-letter aliases were a feature of the prior CLI we don't
        // want to silently drop.
        for (alias, want_locate) in [
            ("l", true),
            ("s", false),
            ("st", false),
            ("d", false),
            ("h", false),
            ("v", false),
        ] {
            let cli = Cli::try_parse_from(["whereami", alias]).unwrap();
            assert!(
                cli.command.is_some(),
                "alias {alias} did not resolve to a subcommand"
            );
            if want_locate {
                assert!(matches!(cli.command, Some(Command::Locate)));
            }
        }
    }

    #[test]
    fn version_flag_is_recognised() {
        let cli = Cli::try_parse_from(["whereami", "--version"]).unwrap();
        assert!(cli.version_flag);
        let cli = Cli::try_parse_from(["whereami", "-V"]).unwrap();
        assert!(cli.version_flag);
    }

    #[test]
    fn help_distinguishes_local_version_flag_from_daemon_subcommand() {
        let help = Cli::command().render_long_help().to_string();
        assert!(
            help.contains("Print this CLI's version locally, without contacting the daemon"),
            "{help}"
        );
        assert!(
            help.contains("Contact the daemon and print both CLI and daemon versions"),
            "{help}"
        );
    }

    #[test]
    fn unknown_subcommand_errors() {
        // Replaces the prior "unknown command: …" branch. clap exits
        // non-zero on parse failure.
        let res = Cli::try_parse_from(["whereami", "wibble"]);
        assert!(res.is_err());
    }

    // --- task-0183: --json raw passthrough ---

    #[test]
    fn json_output_passes_ok_response_through_byte_identically() {
        // An ok:true line must be emitted verbatim: no field absent on the
        // wire (error/address/stale/cached/...) may be invented, and exit
        // code is 0. This is the regression the old re-serialize path had.
        let line = r#"{"ok":true,"v":1,"lat":1.5,"lon":2.5,"accuracy_m":30.0,"sources":3}"#;
        let (out, code) = json_output(Ok(line.to_string()));
        assert_eq!(
            out, line,
            "output must be byte-identical to the daemon line"
        );
        assert_eq!(code, 0);
        for absent in ["error", "address", "stale", "cached", "fetched", "pending"] {
            assert!(
                !out.contains(absent),
                "field {absent:?} was absent on the wire but appeared in --json output"
            );
        }
    }

    #[test]
    fn json_output_error_response_is_verbatim_and_exits_nonzero() {
        // ok:false must pass through unchanged AND exit nonzero. Previously
        // this printed fabricated lat/lon 0.0 (Null Island) and exited 0.
        let line = r#"{"ok":false,"v":1,"error":"scan buffer empty"}"#;
        let (out, code) = json_output(Ok(line.to_string()));
        assert_eq!(out, line);
        assert_ne!(code, 0, "ok:false must exit nonzero");
        assert!(!out.contains("lat"), "must not fabricate a lat field");
        assert!(!out.contains("lon"), "must not fabricate a lon field");
    }

    #[test]
    fn json_output_preserves_unknown_forward_compat_fields() {
        // A field this client version does not know about must survive the
        // passthrough (the old typed re-serialize dropped unknown fields).
        let line = r#"{"ok":true,"v":2,"lat":1.0,"lon":2.0,"future_field":"keepme"}"#;
        let (out, code) = json_output(Ok(line.to_string()));
        assert_eq!(out, line);
        assert_eq!(code, 0);
        assert!(out.contains("future_field"));
    }

    #[test]
    fn json_output_transport_error_is_synthesized_valid_json() {
        // AC #3: daemon unreachable must still yield exactly one valid JSON
        // line on stdout and a nonzero exit.
        let (out, code) = json_output(Err(anyhow::anyhow!("connection refused")));
        assert_ne!(code, 0);
        let v: serde_json::Value = serde_json::from_str(&out).expect("stdout must be valid JSON");
        assert_eq!(v["ok"], serde_json::json!(false));
        assert_eq!(v["error"], serde_json::json!("connection refused"));
    }

    #[test]
    fn json_output_non_json_line_is_synthesized_valid_json() {
        // A broken daemon emitting a non-JSON line must not corrupt stdout;
        // we synthesize an error embedding the (escaped) payload.
        let (out, code) = json_output(Ok("this is not json".to_string()));
        assert_ne!(code, 0);
        let v: serde_json::Value = serde_json::from_str(&out).expect("stdout must be valid JSON");
        assert_eq!(v["ok"], serde_json::json!(false));
        assert!(v["error"].as_str().unwrap().contains("this is not json"));
    }

    #[test]
    fn json_output_missing_ok_field_still_valid_json_and_nonzero() {
        // Valid JSON but no boolean `ok`: pass through byte-identically
        // (stdout stays valid JSON) but treat as failure since ok!=true.
        let line = r#"{"v":1,"note":"weird"}"#;
        let (out, code) = json_output(Ok(line.to_string()));
        assert_eq!(out, line);
        assert_ne!(code, 0);
    }

    #[test]
    fn json_output_control_bytes_never_reach_stdout_raw() {
        // Defense-in-depth: a non-JSON payload carrying terminal escapes
        // must be serde-escaped inside the synthesized error, so stdout is
        // valid JSON and carries no raw ESC (ties task-0183 to task-0198).
        let (out, _code) = json_output(Ok("\u{1b}[2J\u{07}".to_string()));
        assert!(!out.contains('\u{1b}'), "raw ESC must not reach stdout");
        assert!(!out.contains('\u{07}'), "raw BEL must not reach stdout");
        let _v: serde_json::Value = serde_json::from_str(&out).expect("still valid JSON");
    }

    // --- task-0198: control-byte sanitization in the human renderer ---

    /// Assert that a rendered string carries no raw C0 control byte (other
    /// than the renderer's own '\n'), no DEL, and no raw C1 sequence — the
    /// security property of task-0198, checked at the byte level so it
    /// genuinely bites (UTF-8-aware: multibyte chars are not flagged).
    fn assert_no_raw_control_bytes(out: &str) {
        for (i, &b) in out.as_bytes().iter().enumerate() {
            assert!(
                !(b <= 0x1f && b != b'\n'),
                "raw C0 byte {b:#04x} at offset {i} leaked to stdout: {out:?}"
            );
            assert_ne!(b, 0x7f, "raw DEL byte at offset {i} leaked to stdout");
        }
        // C1 controls U+0080..=U+009F encode as UTF-8 0xC2 followed by
        // 0x80..=0x9F. A legitimate char like © (0xC2 0xA9) or an emoji
        // (0xF0-lead) is not matched, so this only catches real C1.
        for w in out.as_bytes().windows(2) {
            assert!(
                !(w[0] == 0xc2 && (0x80..=0x9f).contains(&w[1])),
                "raw C1 sequence {:#04x} {:#04x} leaked to stdout",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn render_scan_escapes_control_bytes_but_keeps_emoji() {
        // SSID mixes the four escapes named by the task (ESC/CR/CSI/BEL)
        // with an emoji, proving escapes are neutralized AND legitimate
        // UTF-8 survives — in a single render pass.
        let resp = whereami_client::ScanResponse {
            ok: true,
            v: 1,
            networks: vec![whereami_client::NetworkEntry {
                bssid: "aa:bb:cc:dd:ee:ff".to_string(),
                ssid: Some("Free\u{1b}[2KWiFi\u{0d}\u{9b}31m\u{07}😀".to_string()),
                signal_dbm: -42,
                channel: Some(6),
            }],
            scan_age_ms: Some(1000),
            scanned_at: Some("2026-07-02T00:00:00Z".to_string()),
            error: None,
        };
        let out = render_scan(&resp, false);
        assert_no_raw_control_bytes(&out);
        // Escapes rendered visibly (not dropped): ESC and CR are the
        // stable, high-confidence forms.
        assert!(
            out.contains("\\u{1b}"),
            "ESC should be escaped, got: {out:?}"
        );
        assert!(out.contains("\\r"), "CR should be escaped");
        // AC #3: legitimate UTF-8 (emoji) survives, and readable SSID text
        // is preserved.
        assert!(out.contains('😀'), "emoji must render unchanged");
        assert!(out.contains("Free") && out.contains("WiFi"));
    }

    #[test]
    fn render_scan_clean_line_is_byte_stable() {
        // sanitize is a no-op on control-free data, so ordinary output is
        // byte-for-byte the fixed layout (guards the println!->String
        // refactor against accidental spacing drift).
        let resp = whereami_client::ScanResponse {
            ok: true,
            v: 1,
            networks: vec![whereami_client::NetworkEntry {
                bssid: "aa:bb:cc:dd:ee:ff".to_string(),
                ssid: Some("HomeNet".to_string()),
                signal_dbm: -55,
                channel: Some(11),
            }],
            scan_age_ms: None,
            scanned_at: None,
            error: None,
        };
        assert_eq!(
            render_scan(&resp, true),
            "aa:bb:cc:dd:ee:ff   -55 dBm  ch11   HomeNet\n"
        );
    }

    #[test]
    fn render_stats_includes_last_clean_shutdown_abort_count_exactly() {
        let resp = whereami_client::StatsResponse {
            ok: true,
            v: 1,
            cached_aps: 42,
            pending_aps: 3,
            not_found_aps: 4,
            db_size_bytes: 49_152,
            api_calls_today: 5,
            api_counter_corruption_denials: 9,
            inflight: 6,
            db_write_failures: 7,
            orphan_tasks_aborted: 8,
            error: None,
        };

        assert_eq!(
            render_stats(&resp),
            "cached:        42\n\
             pending:       3\n\
             not_found:     4\n\
             inflight:      6\n\
             db size:       48 KB\n\
             API today:     5\n\
             API counter corruption denials: 9\n\
             db write fail: 7\n\
             tasks aborted at last clean shutdown: 8\n"
        );
    }

    #[test]
    fn render_locate_clean_output_is_byte_stable() {
        // Same guard as the scan variant: sanitize is a no-op on clean
        // data, so the exact address/coordinate layout is pinned against
        // accidental format drift.
        let resp = whereami_client::LocateResponse {
            ok: true,
            v: 1,
            lat: 55.676098,
            lon: 12.568337,
            accuracy_m: 42.7,
            sources: 4,
            cached: 0,
            fetched: 0,
            pending: 0,
            visible: 0,
            stable: 0,
            address: Some("Rådhuspladsen 1, København".to_string()),
            stale: false,
            age_s: None,
            error: None,
        };
        assert_eq!(
            render_locate(&resp),
            "Rådhuspladsen 1, København\n55.676098, 12.568337  ±42m  (4 sources)\n"
        );
    }

    #[test]
    fn render_locate_escapes_control_bytes_in_address() {
        let resp = whereami_client::LocateResponse {
            ok: true,
            v: 1,
            lat: 1.5,
            lon: 2.5,
            accuracy_m: 10.0,
            sources: 3,
            cached: 0,
            fetched: 0,
            pending: 0,
            visible: 0,
            stable: 0,
            // OSC 0 title-set + BEL: the classic terminal-title injection.
            address: Some("Evil St \u{1b}]0;pwned\u{07}".to_string()),
            stale: false,
            age_s: None,
            error: None,
        };
        let out = render_locate(&resp);
        assert_no_raw_control_bytes(&out);
        assert!(
            out.contains("\\u{1b}"),
            "ESC should be escaped, got: {out:?}"
        );
        assert!(out.contains("Evil St"));
    }

    #[test]
    fn sanitize_is_identity_on_printable_and_multibyte() {
        assert_eq!(sanitize("plain ascii 123"), "plain ascii 123");
        assert_eq!(sanitize("café 北京 😀"), "café 北京 😀");
    }
}
