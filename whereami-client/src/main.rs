use std::process;
use whereami_client::WhereAmIClient;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("locate");
    let json = args.iter().any(|a| a == "--json");

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
        "scan" | "s" => match client.scan() {
            Ok(resp) if json => {
                println!("{}", serde_json::to_string(&resp).unwrap());
            }
            Ok(resp) if !resp.ok => {
                eprintln!("{}", resp.error.as_deref().unwrap_or("unknown error"));
                process::exit(1);
            }
            Ok(resp) => {
                for net in &resp.networks {
                    println!(
                        "{:17}  {:>4} dBm  ch{:<3}  {}",
                        net.bssid,
                        net.signal_dbm,
                        net.channel.map(|c| c.to_string()).unwrap_or("-".into()),
                        net.ssid.as_deref().unwrap_or("<hidden>")
                    );
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
        "help" | "-h" | "--help" => {
            println!("whereami — Wi-Fi geolocation CLI");
            println!();
            println!("Usage: whereami [command] [--json]");
            println!();
            println!("Commands:");
            println!("  locate (default)  Where am I?");
            println!("  scan              Visible Wi-Fi networks");
            println!("  stats             Cache statistics");
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
