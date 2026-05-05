//! whereamid: Wi-Fi geolocation daemon.
//!
//! Scans Wi-Fi in the background, caches AP positions in SQLite,
//! and answers "where am I?" queries over TCP + JSON-lines.

mod apple;
#[allow(dead_code)]
mod beacondb;
mod config;
mod db;
mod debounce;
mod nominatim;
mod pending;
mod resolver;
mod scanner;
mod server;
mod trilaterate;
mod wigle;

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::signal;
use tokio::time::{sleep, Duration, Instant};
use tracing::{error, info};

use config::{load_config_file, Args};
use db::Database;
use debounce::Debouncer;
use server::DaemonState;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    info!("whereamid starting");
    info!("bind: {}", args.bind);
    info!("db: {}", args.db.display());
    info!("interface: {}", args.interface);

    // Ensure DB directory exists
    if let Some(parent) = args.db.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating db directory {}", parent.display()))?;
    }

    // Load secrets from config file
    let config_file = load_config_file(&args.config)?;

    // Open database
    let db = Database::open(&args.db).context("opening database")?;

    // Initialize shared state
    let debouncer = Debouncer::new(args.debounce_window, args.debounce_threshold);
    let wigle_client =
        wigle::WigleClient::new(&config_file.wigle.api_user, &config_file.wigle.api_key);
    let beacondb_client = beacondb::BeaconDbClient::new(config_file.beacondb.enabled);

    if !wigle_client.is_configured() {
        tracing::warn!("WiGLE credentials not configured - remote lookups disabled");
    }

    let state = Arc::new(DaemonState {
        db: std::sync::Mutex::new(db),
        debouncer: tokio::sync::Mutex::new(debouncer),
        args: args.clone(),
        wigle: wigle_client,
        beacondb: beacondb_client,
        apple: apple::AppleClient::new(),
        nominatim: nominatim::NominatimClient::new(),
    });

    // Spawn background scan loop
    let scan_state = Arc::clone(&state);
    tokio::spawn(async move {
        run_scan_loop(scan_state).await;
    });

    // Spawn pending queue drain task
    let pending_state = Arc::clone(&state);
    tokio::spawn(async move {
        pending::run_pending_drain(pending_state).await;
    });

    // Spawn TCP server
    let server_state = Arc::clone(&state);
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server::run_server(server_state).await {
            error!("server error: {e}");
        }
    });

    // Wait for shutdown signal
    tokio::select! {
        _ = signal::ctrl_c() => {
            info!("received SIGINT, shutting down");
        }
        _ = server_handle => {
            error!("server task exited unexpectedly");
        }
    }

    info!("whereamid stopped");
    Ok(())
}

/// Background scan loop with fast/slow phase.
async fn run_scan_loop(state: Arc<DaemonState>) {
    let start = Instant::now();
    let fast_duration = Duration::from_secs(state.args.scan_fast_duration);
    let fast_interval = Duration::from_secs(state.args.scan_interval_fast);
    let slow_interval = Duration::from_secs(state.args.scan_interval_slow);

    info!(
        "scan loop started (fast: {}s for {}s, then slow: {}s)",
        state.args.scan_interval_fast, state.args.scan_fast_duration, state.args.scan_interval_slow
    );

    loop {
        let interval = if start.elapsed() < fast_duration {
            fast_interval
        } else {
            slow_interval
        };

        match scanner::wifi_scan(&state.args.interface).await {
            Ok(networks) => {
                let count = networks.len();
                let sample = scanner::scan_to_sample(&networks);
                let mut debouncer = state.debouncer.lock().await;
                debouncer.push_scan(sample);
                let stable = debouncer.stable_bssids();
                let stable_count = stable.len();
                let sample_count = debouncer.sample_count();
                drop(debouncer);

                info!(
                    "scan: {} networks, {} stable ({} samples in buffer)",
                    count, stable_count, sample_count
                );

                // Proactively resolve newly-stable APs in the background
                if !stable.is_empty() {
                    let resolve_state = Arc::clone(&state);
                    let stable_vec: Vec<String> = stable.into_iter().collect();
                    tokio::spawn(async move {
                        resolver::resolve_background(&stable_vec, &resolve_state).await;
                    });
                }
            }
            Err(e) => {
                tracing::warn!("wifi scan failed: {e}");
            }
        }

        sleep(interval).await;
    }
}
