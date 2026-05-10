//! whereamid: Wi-Fi geolocation daemon.
//!
//! Scans Wi-Fi in the background, caches AP positions in SQLite,
//! and answers "where am I?" queries over TCP + JSON-lines.

mod apple;
mod config;
mod db;
mod debounce;
mod geo;
mod history;
mod http;
mod nominatim;
mod pending;
mod provider;
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
    // Honor RUST_LOG (e.g. RUST_LOG=whereamid=debug) and default to "info"
    // when unset. Requires the env-filter feature on tracing-subscriber
    // (task-0054); without it the default fmt::init silently drops debug!.
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    // Cross-field validation runs before any side effects (DB open,
    // signals, threads). Bad combinations produce a clean error message
    // and a non-zero exit, not a runtime assert deep inside Debouncer.
    args.validate().context("invalid CLI arguments")?;
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

    // Rehydrate last_fix from disk so the daemon can answer "where am I"
    // immediately after a restart, before the first scan-and-resolve cycle
    // completes. A malformed or out-of-range timestamp is treated as "no
    // last fix" — better to skip than to crash on startup.
    let initial_last_fix = match db.get_last_fix() {
        Ok(Some(row)) => match server::LastFix::try_from(row) {
            Ok(fix) => Some(fix),
            Err(e) => {
                tracing::warn!("discarding persisted last_fix: {e}");
                None
            }
        },
        Ok(None) => None,
        Err(e) => {
            tracing::warn!("failed to read persisted last_fix: {e}");
            None
        }
    };
    if initial_last_fix.is_some() {
        info!("rehydrated last_fix from disk");
    }

    // Initialize shared state
    let debouncer = Debouncer::new(args.debounce_window, args.debounce_threshold);
    let http_timeout = Duration::from_secs(args.http_timeout_secs);
    let nominatim_timeout = Duration::from_secs(args.nominatim_timeout_secs);
    let wigle_client = wigle::WigleClient::with_timeout(
        &config_file.wigle.api_user,
        &config_file.wigle.api_key,
        http_timeout,
    );

    if !wigle_client.is_configured() {
        tracing::warn!("WiGLE credentials not configured - remote lookups disabled");
    }

    let state = Arc::new(DaemonState {
        db: std::sync::Mutex::new(db),
        debouncer: tokio::sync::Mutex::new(debouncer),
        apple: apple::AppleClient::with_timeout(http_timeout),
        nominatim: nominatim::NominatimClient::with_timeout(nominatim_timeout),
        address_cache: std::sync::Mutex::new(server::AddressCache::with_ttl_days(
            args.address_cache_ttl_days,
        )),
        args: args.clone(),
        wigle: wigle_client,
        last_fix: tokio::sync::Mutex::new(initial_last_fix),
        inflight: std::sync::Mutex::new(std::collections::HashSet::new()),
        db_write_failures: std::sync::atomic::AtomicU64::new(0),
        shutdown: tokio::sync::Notify::new(),
    });

    // Background tasks. Each respects state.shutdown via tokio::select!
    // around its sleep, so they exit at the next iteration boundary rather
    // than being cut off mid-await on SIGTERM (task-0075).
    let scan_state = Arc::clone(&state);
    let scan_handle = tokio::spawn(async move {
        run_scan_loop(scan_state).await;
    });

    let pending_state = Arc::clone(&state);
    let pending_handle = tokio::spawn(async move {
        pending::run_pending_drain(pending_state).await;
    });

    // History-prune task: drop fixes older than retention_days every 24h.
    // The first sweep runs after one full interval so the daemon's startup
    // is not slowed by a potentially large DELETE.
    let history_state = Arc::clone(&state);
    let history_handle = tokio::spawn(async move {
        let interval = Duration::from_secs(24 * 60 * 60);
        loop {
            tokio::select! {
                _ = sleep(interval) => {},
                _ = history_state.shutdown.notified() => break,
            }
            let retention = history_state.args.history_retention_days;
            let pruned = {
                let db = server::lock_db(&history_state);
                db.prune_fixes(retention)
            };
            match pruned {
                Ok(0) => {}
                Ok(n) => info!("history prune: removed {n} fix rows older than {retention}d"),
                Err(e) => {
                    tracing::warn!("history prune failed: {e}");
                    history_state.record_db_failure();
                }
            }
        }
    });

    let server_state = Arc::clone(&state);
    let mut server_handle = tokio::spawn(async move {
        if let Err(e) = server::run_server(server_state).await {
            error!("server error: {e}");
        }
    });

    // Wait for shutdown signal. Listen for both SIGINT (Ctrl-C) and SIGTERM
    // (systemctl stop, docker stop, etc.). Without a SIGTERM handler systemd
    // would SIGKILL the daemon after the stop timeout, potentially mid-write
    // to SQLite or mid-API-call.
    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
        .context("installing SIGTERM handler")?;
    tokio::select! {
        _ = signal::ctrl_c() => {
            info!("received SIGINT, shutting down");
        }
        _ = sigterm.recv() => {
            info!("received SIGTERM, shutting down");
        }
        _ = &mut server_handle => {
            error!("server task exited unexpectedly");
        }
    }

    // task-0075: cooperative drain. Notify all background loops to exit at
    // their next iteration, then wait up to 5s for them to finish. Tasks
    // that are between iterations (sleep) return promptly; tasks mid-DB-
    // write return after the write completes. After the timeout we abort
    // anything still running.
    state.shutdown.notify_waiters();
    server_handle.abort();
    let drain_timeout = Duration::from_secs(5);
    let drain = async {
        let _ = scan_handle.await;
        let _ = pending_handle.await;
        let _ = history_handle.await;
        let _ = server_handle.await;
    };
    if tokio::time::timeout(drain_timeout, drain).await.is_err() {
        tracing::warn!(
            "background tasks did not drain within {}s; aborting",
            drain_timeout.as_secs()
        );
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

        // Cooperative shutdown (task-0075): break out at the next iteration
        // boundary when main() calls state.shutdown.notify_waiters().
        tokio::select! {
            _ = sleep(interval) => {},
            _ = state.shutdown.notified() => break,
        }
    }
}
