use anyhow::Result;
use serde::Serialize;
use std::process;
use whereami_client::{DaemonResponse, WhereAmIClient};

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
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("locate");
    let json = args.iter().any(|a| a == "--json");
    let no_scan_time = args.iter().any(|a| a == "--scan-time=no");

    let client = WhereAmIClient::default_addr();

    match cmd {
        "locate" | "l" => dispatch(client.locate(), json, |resp| {
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
        }),
        "scan" | "s" => dispatch(client.scan(), json, |resp| {
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
        }),
        "stats" | "st" => dispatch(client.stats(), json, |resp| {
            println!("cached:        {}", resp.cached_aps);
            println!("pending:       {}", resp.pending_aps);
            println!("not_found:     {}", resp.not_found_aps);
            println!("inflight:      {}", resp.inflight);
            println!("db size:       {} KB", resp.db_size_bytes / 1024);
            println!("API today:     {}", resp.api_calls_today);
            println!("db write fail: {}", resp.db_write_failures);
        }),
        "debug" | "d" => dispatch(client.debug(), json, |resp| {
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
        }),
        "history" | "h" => {
            // First non-flag arg after "history" is the range (e.g. "7d");
            // default to 7 days. --from / --to take RFC3339 timestamps.
            let mut range: Option<String> = None;
            let mut from: Option<String> = None;
            let mut to: Option<String> = None;
            let rest = &args[2..];
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--from" => {
                        i += 1;
                        if i >= rest.len() {
                            fatal("--from requires an RFC3339 timestamp");
                        }
                        from = Some(rest[i].clone());
                    }
                    "--to" => {
                        i += 1;
                        if i >= rest.len() {
                            fatal("--to requires an RFC3339 timestamp");
                        }
                        to = Some(rest[i].clone());
                    }
                    "--json" | "--scan-time=no" => {}
                    other if !other.starts_with("--") && range.is_none() => {
                        range = Some(other.to_string());
                    }
                    other => fatal(&format!("unknown history arg: {other}")),
                }
                i += 1;
            }
            // Default range = 7d if no explicit range/from-to.
            if range.is_none() && from.is_none() && to.is_none() {
                range = Some("7d".into());
            }
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
        "version" | "v" | "--version" | "-V" => {
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
        "help" | "-h" | "--help" => {
            println!("whereami — Wi-Fi geolocation CLI");
            println!();
            println!("Usage: whereami [command] [--json]");
            println!();
            println!("Commands:");
            println!("  locate (default)  Where am I?");
            println!("  scan              Visible Wi-Fi networks");
            println!("  stats             Cache statistics");
            println!("  debug             Daemon debug state");
            println!("  history [range]   Stay-point segments (default 7d; e.g. 24h, 7d, 1w)");
            println!("  version           Print CLI and daemon versions");
            println!();
            println!("Flags:");
            println!("  --json            Output raw JSON");
            println!("  --from RFC3339    Absolute start (history)");
            println!("  --to   RFC3339    Absolute end (history)");
        }
        other => {
            eprintln!("unknown command: {other}");
            eprintln!("try: whereami help");
            process::exit(1);
        }
    }
}
