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
        shutdown: tokio_util::sync::CancellationToken::new(),
        tasks: std::sync::Mutex::new(tokio::task::JoinSet::new()),
        orphan_tasks_aborted: std::sync::atomic::AtomicU64::new(0),
    });

    // Background tasks. Each selects its work/sleep against
    // state.shutdown.cancelled() (level-triggered, task-0190), so they exit at
    // the next await boundary on SIGTERM rather than missing the wakeup.
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
            // Level-triggered shutdown (task-0190): `biased` checks
            // cancellation first, so a cancel that lands during the interval
            // wakes the loop immediately instead of waiting out the full 24h.
            tokio::select! {
                biased;
                _ = history_state.shutdown.cancelled() => break,
                _ = sleep(interval) => {}
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
    let server_exited = tokio::select! {
        _ = signal::ctrl_c() => {
            info!("received SIGINT, shutting down");
            false
        }
        _ = sigterm.recv() => {
            info!("received SIGTERM, shutting down");
            false
        }
        res = &mut server_handle => {
            // The server task ended on its own (e.g. bind failure). This arm
            // consumes the JoinHandle's output; it must NOT be awaited again
            // or tokio panics on the second poll (task-0195).
            match res {
                Ok(()) => error!("server task exited unexpectedly"),
                Err(e) => error!("server task join error: {e}"),
            }
            true
        }
    };

    // task-0190: level-triggered cancel. Sticky, so a loop that is mid-
    // iteration (not parked in its select!) still observes it at the next
    // await instead of missing the wakeup and running forever.
    state.shutdown.cancel();

    // Collect the loop handles to drain. The server is special-cased for
    // task-0195: if it already exited above, its handle is spent — drop it
    // without a second poll; otherwise abort it (accept() has no state to
    // flush) and drain it with the rest.
    let mut named = vec![scan_handle, pending_handle, history_handle];
    if !server_exited {
        server_handle.abort();
        named.push(server_handle);
    }

    // Take the tracked-spawn JoinSet out of shared state so we can drive it to
    // completion here (a std::sync::Mutex guard cannot be held across await).
    let tracked = std::mem::take(&mut *state.tasks.lock().unwrap_or_else(|e| e.into_inner()));

    // task-0190/0102: wait up to the budget, then force-abort survivors and
    // count them so the leak is observable via stats.
    let aborted = run_shutdown_drain(Duration::from_secs(5), named, tracked).await;
    if aborted > 0 {
        state
            .orphan_tasks_aborted
            .fetch_add(aborted, std::sync::atomic::Ordering::Relaxed);
    }

    info!("whereamid stopped");
    Ok(())
}

/// Drive the cooperative shutdown drain (task-0190/0102/0195).
///
/// The caller has already signalled `state.shutdown.cancel()`. This waits up
/// to `budget` for the `named` loop handles and the `tracked` JoinSet to
/// finish cooperatively; on timeout it force-aborts everything still running
/// and returns the number of tasks that had to be aborted (named loops +
/// tracked spawns), which the caller records into `orphan_tasks_aborted`.
///
/// Every JoinHandle is polled at most once (task-0195): the graceful future
/// consumes the `named` handles, and the pre-extracted `AbortHandle`s are the
/// only thing touched on the timeout path — the handles are never re-awaited.
async fn run_shutdown_drain(
    budget: Duration,
    named: Vec<tokio::task::JoinHandle<()>>,
    mut tracked: tokio::task::JoinSet<()>,
) -> u64 {
    let named_aborts: Vec<tokio::task::AbortHandle> =
        named.iter().map(|h| h.abort_handle()).collect();

    // Graceful phase. `named` is moved in and consumed (owned `JoinHandle`s,
    // awaited once each); `tracked` is borrowed so it survives a timeout for
    // the force-abort + count below.
    let graceful = async {
        for h in named {
            let _ = h.await;
        }
        while tracked.join_next().await.is_some() {}
    };

    if tokio::time::timeout(budget, graceful).await.is_ok() {
        // Everything drained cooperatively; nothing to abort.
        return 0;
    }

    tracing::warn!(
        "background tasks did not drain within {}s; aborting",
        budget.as_secs()
    );

    let mut aborted: u64 = 0;
    for a in &named_aborts {
        if !a.is_finished() {
            a.abort();
            aborted += 1;
        }
    }
    // JoinSet::len() is the number of not-yet-joined tasks; on timeout these
    // are the survivors. Abort them and reap so the aborts are observed.
    aborted += tracked.len() as u64;
    tracked.abort_all();
    while tracked.join_next().await.is_some() {}

    aborted
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

        // Race the scan against cancellation: a SIGTERM landing mid-scan
        // drops the wifi_scan future at this await (task-0190) instead of
        // running it to completion. scanner sets kill_on_drop on its nmcli/iw
        // children, so a dropped scan leaves no orphaned subprocess.
        let scan_result = tokio::select! {
            biased;
            _ = state.shutdown.cancelled() => break,
            r = scanner::wifi_scan(&state.args.interface) => r,
        };

        match scan_result {
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

                // Proactively resolve newly-stable APs in the background.
                // task-0190 AC#2: never start new resolver work once shutdown
                // has been signalled.
                if !stable.is_empty() && !state.shutdown.is_cancelled() {
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

        // Sleep between scans, waking immediately on shutdown (task-0190).
        tokio::select! {
            biased;
            _ = state.shutdown.cancelled() => break,
            _ = sleep(interval) => {}
        }
    }
}

#[cfg(test)]
mod shutdown_drain_tests {
    //! Deterministic (paused-clock) tests for the shutdown drain
    //! (task-0190/0102/0195). No wall-clock waits: the runtime auto-advances
    //! virtual time to the earliest pending timer, so the budget timeout fires
    //! exactly when expected.
    use super::run_shutdown_drain;
    use tokio::task::JoinSet;
    use tokio::time::{Duration, Instant};

    /// Fast path: every named handle and tracked task finishes cooperatively
    /// within the budget, so nothing is force-aborted and the count is 0.
    #[tokio::test(start_paused = true)]
    async fn drain_returns_zero_when_all_finish() {
        let named = vec![tokio::spawn(async {}), tokio::spawn(async {})];
        let mut tracked = JoinSet::new();
        tracked.spawn(async {});
        tracked.spawn(async {});
        let aborted = run_shutdown_drain(Duration::from_secs(5), named, tracked).await;
        assert_eq!(aborted, 0);
    }

    /// task-0102 AC#4: five tracked tasks that sleep past the budget are all
    /// force-aborted and counted, and the drain returns at ~budget rather than
    /// waiting out the 10s sleeps.
    #[tokio::test(start_paused = true)]
    async fn drain_aborts_five_stuck_tracked_tasks() {
        let mut tracked = JoinSet::new();
        for _ in 0..5 {
            tracked.spawn(async {
                tokio::time::sleep(Duration::from_secs(10)).await;
            });
        }
        let start = Instant::now();
        let aborted = run_shutdown_drain(Duration::from_secs(1), Vec::new(), tracked).await;
        let elapsed = start.elapsed();
        assert_eq!(
            aborted, 5,
            "all five stuck tracked tasks must be force-aborted and counted"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "drain must return at ~budget, not wait out the 10s sleeps (elapsed {elapsed:?})"
        );
    }

    /// task-0190 AC#3: a stuck *named* loop handle is force-aborted via its
    /// pre-extracted AbortHandle and counted.
    #[tokio::test(start_paused = true)]
    async fn drain_aborts_stuck_named_handle() {
        let named = vec![tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(10)).await;
        })];
        let aborted = run_shutdown_drain(Duration::from_secs(1), named, JoinSet::new()).await;
        assert_eq!(aborted, 1);
    }

    /// Only survivors are counted: a task that finished before the timeout was
    /// reaped and does not count; the still-running one does.
    #[tokio::test(start_paused = true)]
    async fn drain_counts_only_survivors() {
        let mut tracked = JoinSet::new();
        tracked.spawn(async {}); // finishes immediately
        tracked.spawn(async {
            tokio::time::sleep(Duration::from_secs(10)).await;
        }); // still running at the budget
        let aborted = run_shutdown_drain(Duration::from_secs(1), Vec::new(), tracked).await;
        assert_eq!(
            aborted, 1,
            "only the still-running task counts; the finished one was reaped"
        );
    }
}
