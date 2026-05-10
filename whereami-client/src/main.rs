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
//! - `--version` / `-V` print the same cli+daemon banner the prior
//!   `version` subcommand emitted; clap's own version flag is suppressed.

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::process;
use whereami_client::{DaemonResponse, WhereAmIClient};

/// `whereami` — Wi-Fi geolocation CLI.
///
/// Talks JSON over TCP to the local whereamid daemon (default
/// 127.0.0.1:4747). Output is human-readable by default; pass `--json`
/// to get the raw daemon response on stdout.
#[derive(Parser, Debug)]
#[command(
    name = "whereami",
    bin_name = "whereami",
    about = "Wi-Fi geolocation CLI (talks to whereamid)",
    long_about = None,
    // We provide our own --version that also queries the daemon, so
    // disable clap's auto-generated one. The `version` subcommand and
    // top-level --version flag both route to handle_version().
    disable_version_flag = true,
    arg_required_else_help = false,
)]
struct Cli {
    /// Print the raw JSON daemon response instead of the human renderer.
    #[arg(long, global = true)]
    json: bool,

    /// Print CLI + daemon version and exit. Equivalent to the `version`
    /// subcommand. Provided as a flag because `whereami --version` is
    /// a near-universal convention.
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
        /// Suppress the "scanned: <ts> (Ns ago)" header line. Defaults
        /// to `yes` (i.e. show the header). The legacy invocation
        /// `whereami scan --scan-time=no` still works.
        #[arg(long, value_enum, default_value_t = YesNo::Yes)]
        scan_time: YesNo,
    },

    /// Cache, pending, and rate-limit statistics.
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

    /// Print CLI and daemon versions.
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

/// Common dispatch ladder: --json prints raw, ok=false fatals with the
/// daemon's error, ok=true delegates to render. Replaces six near-
/// identical match blocks in the CLI (task-0058).
fn dispatch<T: Serialize + DaemonResponse>(result: Result<T>, json: bool, render: impl FnOnce(&T)) {
    match result {
        Ok(resp) if json => println!(
            "{}",
            serde_json::to_string(&resp).expect("response struct must serialize")
        ),
        Ok(resp) if !resp.is_ok() => fatal(resp.error().unwrap_or("unknown error")),
        Ok(resp) => render(&resp),
        Err(e) => fatal(&format!("error: {e}")),
    }
}

fn main() {
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
    let json = cli.json;
    let client = WhereAmIClient::default_addr();

    // --version short-circuits all other subcommands. We render the same
    // banner the `version` subcommand produces.
    if cli.version_flag {
        return run_version(&client, json);
    }

    // No subcommand -> default to locate (backwards compatible with the
    // pre-clap CLI that did `cmd = args.get(1).unwrap_or("locate")`).
    let command = cli.command.unwrap_or(Command::Locate);

    match command {
        Command::Locate => run_locate(&client, json),
        Command::Scan { scan_time } => run_scan(&client, json, scan_time == YesNo::No),
        Command::Stats => run_stats(&client, json),
        Command::Debug => run_debug(&client, json),
        Command::History { range, from, to } => run_history(&client, json, range, from, to),
        Command::Version => run_version(&client, json),
    }
}

fn run_locate(client: &WhereAmIClient, json: bool) {
    dispatch(client.locate(), json, |resp| {
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
        if let Some(addr) = resp.address.as_deref() {
            if !addr.is_empty() {
                println!("{addr}");
            }
        }
        println!(
            "{:.6}, {:.6}  ±{}m  ({} sources)",
            resp.lat, resp.lon, resp.accuracy_m as i64, resp.sources,
        );
    });
}

fn run_scan(client: &WhereAmIClient, json: bool, no_scan_time: bool) {
    dispatch(client.scan(), json, |resp| {
        if !no_scan_time {
            let age_ms = resp.scan_age_ms.unwrap_or(0);
            let age_s = age_ms / 1000;
            let scanned_at = resp.scanned_at.as_deref().unwrap_or("?");
            println!("scanned: {scanned_at}  ({age_s}s ago)");
            println!();
        }
        for net in &resp.networks {
            println!(
                "{:17}  {:>4} dBm  ch{:<3}  {}",
                net.bssid,
                net.signal_dbm,
                net.channel
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".into()),
                net.ssid.as_deref().unwrap_or("<hidden>")
            );
        }
    });
}

fn run_stats(client: &WhereAmIClient, json: bool) {
    dispatch(client.stats(), json, |resp| {
        println!("cached:        {}", resp.cached_aps);
        println!("pending:       {}", resp.pending_aps);
        println!("not_found:     {}", resp.not_found_aps);
        println!("inflight:      {}", resp.inflight);
        println!("db size:       {} KB", resp.db_size_bytes / 1024);
        println!("API today:     {}", resp.api_calls_today);
        println!("db write fail: {}", resp.db_write_failures);
    });
}

fn run_debug(client: &WhereAmIClient, json: bool) {
    dispatch(client.debug(), json, |resp| {
        let cli_rev = env!("GIT_REV");
        let daemon_rev = resp.daemon_rev.as_deref().unwrap_or("?");
        println!("cli: {cli_rev}  daemon: {daemon_rev}");
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
                marker, b.bssid, signal, b.seen, b.needed, b.db_status,
            );
        }
        println!();
        println!("* = stable  seen/needed = debounce count/threshold");
    });
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
    dispatch(client.history(range, from, to), json, |resp| {
        println!("range: {} -> {}", resp.from, resp.to);
        println!("segments: {}", resp.segments.len());
        for seg in &resp.segments {
            let dur_min = seg.duration_secs / 60;
            println!(
                "  {} -> {}  {:.6},{:.6}  ±{:.0}m  {} fixes  ({}m)",
                seg.start_rfc3339,
                seg.end_rfc3339,
                seg.centroid_lat,
                seg.centroid_lon,
                seg.mean_accuracy_m,
                seg.n_fixes,
                dur_min
            );
        }
    });
}

fn run_version(client: &WhereAmIClient, json: bool) {
    let cli_version = env!("CARGO_PKG_VERSION");
    let cli_rev = env!("GIT_REV");
    match client.version() {
        Ok(resp) if json => {
            println!("{}", serde_json::to_string(&resp).unwrap());
        }
        Ok(resp) if !resp.ok => {
            println!("cli:    {cli_version} ({cli_rev})");
            eprintln!(
                "daemon: {}",
                resp.error.as_deref().unwrap_or("unknown error")
            );
            process::exit(1);
        }
        Ok(resp) => {
            let daemon_version = resp.version.as_deref().unwrap_or("?");
            let daemon_rev = resp.git_rev.as_deref().unwrap_or("?");
            println!("cli:    {cli_version} ({cli_rev})");
            println!("daemon: {daemon_version} ({daemon_rev})");
        }
        Err(e) => {
            println!("cli:    {cli_version} ({cli_rev})");
            eprintln!("daemon: error: {e}");
            process::exit(1);
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
    fn unknown_subcommand_errors() {
        // Replaces the prior "unknown command: …" branch. clap exits
        // non-zero on parse failure.
        let res = Cli::try_parse_from(["whereami", "wibble"]);
        assert!(res.is_err());
    }
}
