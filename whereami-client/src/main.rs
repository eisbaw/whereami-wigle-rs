use std::process;
use whereami_client::WhereAmIClient;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("locate");
    let json = args.iter().any(|a| a == "--json");
    let no_scan_time = args.iter().any(|a| a == "--scan-time=no");

    let client = WhereAmIClient::default_addr();

    match cmd {
        "locate" | "l" => match client.locate() {
            Ok(resp) if json => {
                println!("{}", serde_json::to_string(&resp).unwrap());
            }
            Ok(resp) if !resp.ok => {
                eprintln!("{}", resp.error.as_deref().unwrap_or("unknown error"));
                process::exit(1);
            }
            Ok(resp) => {
                if let Some(addr) = &resp.address {
                    println!("{}", addr);
                }
                println!(
                    "{:.6}, {:.6}  ±{}m  ({} sources, {} cached)",
                    resp.lat, resp.lon, resp.accuracy_m as i64, resp.sources, resp.cached
                );
            }
            Err(e) => {
                eprintln!("error: {e}");
                process::exit(1);
            }
        },
        "scan" | "s" => match client.raw_command(r#"{"cmd":"scan"}"#) {
            Ok(resp) if json => println!("{resp}"),
            Ok(resp) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
                    if v["ok"].as_bool() != Some(true) {
                        eprintln!("{}", v["error"].as_str().unwrap_or("unknown error"));
                        process::exit(1);
                    }
                    if !no_scan_time {
                        let age_ms = v["scan_age_ms"].as_u64().unwrap_or(0);
                        let age_s = age_ms / 1000;
                        let scanned_at = v["scanned_at"].as_str().unwrap_or("?");
                        println!("scanned: {}  ({}s ago)", scanned_at, age_s);
                        println!();
                    }
                    if let Some(nets) = v["networks"].as_array() {
                        for net in nets {
                            println!(
                                "{:17}  {:>4} dBm  ch{:<3}  {}",
                                net["bssid"].as_str().unwrap_or("?"),
                                net["signal_dbm"].as_i64().unwrap_or(0),
                                net["channel"]
                                    .as_i64()
                                    .map(|c| c.to_string())
                                    .unwrap_or("-".into()),
                                net["ssid"].as_str().unwrap_or("<hidden>")
                            );
                        }
                    }
                } else {
                    println!("{resp}");
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                process::exit(1);
            }
        },
        "stats" | "st" => match client.stats() {
            Ok(resp) if json => {
                println!("{}", serde_json::to_string(&resp).unwrap());
            }
            Ok(resp) if !resp.ok => {
                eprintln!("{}", resp.error.as_deref().unwrap_or("unknown error"));
                process::exit(1);
            }
            Ok(resp) => {
                println!("cached:    {}", resp.cached_aps);
                println!("pending:   {}", resp.pending_aps);
                println!("not_found: {}", resp.not_found_aps);
                println!("db size:   {} KB", resp.db_size_bytes / 1024);
                println!("API today: {}", resp.api_calls_today);
            }
            Err(e) => {
                eprintln!("error: {e}");
                process::exit(1);
            }
        },
        "debug" | "d" => {
            let client_raw = WhereAmIClient::default_addr();
            match client_raw.raw_command(r#"{"cmd":"debug"}"#) {
                Ok(resp) if json => println!("{resp}"),
                Ok(resp) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
                        let daemon_rev = v["daemon_rev"].as_str().unwrap_or("?");
                        let cli_rev = env!("GIT_REV");
                        println!("cli: {}  daemon: {}", cli_rev, daemon_rev);
                        println!();
                        let age = v["scan_age_ms"].as_u64().unwrap_or(0);
                        let samples = v["samples_in_buffer"].as_u64().unwrap_or(0);
                        let visible = v["visible"].as_u64().unwrap_or(0);
                        let stable = v["stable"].as_u64().unwrap_or(0);
                        println!(
                            "scan age: {}ms  samples: {}  visible: {}  stable: {}",
                            age, samples, visible, stable
                        );
                        println!();
                        if let Some(bssids) = v["bssids"].as_array() {
                            for b in bssids {
                                let seen = b["seen"].as_u64().unwrap_or(0);
                                let needed = b["needed"].as_u64().unwrap_or(0);
                                let is_stable = b["is_stable"].as_bool().unwrap_or(false);
                                let marker = if is_stable { "*" } else { " " };
                                println!(
                                    "{}{:17}  {:>4} dBm  {}/{}  {}",
                                    marker,
                                    b["bssid"].as_str().unwrap_or("?"),
                                    b["signal_dbm"].as_i64().unwrap_or(0),
                                    seen,
                                    needed,
                                    b["db_status"].as_str().unwrap_or("?"),
                                );
                            }
                            println!();
                            println!("* = stable  seen/needed = debounce count/threshold");
                        }
                    } else {
                        println!("{resp}");
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
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
            println!();
            println!("Flags:");
            println!("  --json            Output raw JSON");
        }
        other => {
            eprintln!("unknown command: {other}");
            eprintln!("try: whereami help");
            process::exit(1);
        }
    }
}
