//! whereamid: Wi-Fi geolocation daemon.
//!
//! Scans Wi-Fi in the background, caches AP positions in SQLite, and answers
//! "where am I?" queries over a JSON-lines control channel. The canonical
//! transport is a Unix domain socket gated by filesystem permissions (ADR-003);
//! TCP is opt-in and unauthenticated (`--bind`).

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

use std::os::unix::io::FromRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::signal;
use tokio::time::{sleep, Duration, Instant};
use tracing::{error, info, warn};

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

    // Install the termination-signal handlers BEFORE any slow startup work
    // (DB open + migrations, socket bind). tokio's `signal()` registers the
    // OS-level handler at call time and latches a pending signal in a watch
    // channel, so a SIGTERM/SIGINT that lands during startup is not fatal: the
    // process finishes startup, then the select! below observes the latched
    // signal and runs the normal shutdown drain — instead of the kernel
    // terminating the daemon by default disposition (signal 15) mid-migration.
    // The signal is deferred until startup completes, not acted on immediately;
    // for this daemon's tiny migrations that is well inside systemd's stop
    // timeout. Installing early also makes the readiness handshake in
    // tests/shutdown.rs sound: once the socket is bound the handler is already
    // installed (task-0237).
    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
        .context("installing SIGTERM handler")?;
    let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt())
        .context("installing SIGINT handler")?;

    info!("whereamid starting");
    match &args.bind {
        Some(b) => info!("bind (TCP, opt-in, unauthenticated): {b}"),
        None => info!("bind: TCP disabled (unix control socket only)"),
    }
    info!("db: {}", args.db.display());
    info!("interface: {}", args.interface);

    // Ensure the DB's parent directory exists and is owner-only (0700). We
    // create it at a tight mode ourselves rather than relying solely on the
    // systemd unit's StateDirectoryMode/UMask, so manual and --db-relocated
    // runs are protected too (task-0197). A pre-existing directory the daemon
    // did NOT create is left untouched (never chmod a possibly-shared parent
    // like /tmp) but warned about when it is group/other-accessible.
    if let Some(parent) = args.db.parent() {
        // parent() is "" for a bare filename; that means CWD, which already
        // exists and is not ours to re-permission.
        if !parent.as_os_str().is_empty() {
            ensure_private_dir(parent)?;
            warn_if_dir_accessible(parent);
        }
    }

    // Load secrets from config file
    let config_file = load_config_file(&args.config)?;

    // Open database
    let db = Database::open(&args.db).context("opening database")?;

    // Rehydrate last_fix from disk so the daemon can answer "where am I"
    // immediately after a restart, before the first scan-and-resolve cycle
    // completes. A malformed timestamp or non-finite coordinate is treated as
    // "no last fix" — better to discard it than serve a poisoned stale fix.
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

    // Rehydrate the last clean shutdown's forced-abort count so the stats
    // endpoint can actually report it (task-0102 review): the in-memory bump
    // happens only after the server socket has closed, so without this
    // persist/reload cycle no client could ever observe a non-zero value.
    let orphans_last_shutdown = load_orphan_count(&db);
    if orphans_last_shutdown > 0 {
        info!("last shutdown force-aborted {orphans_last_shutdown} tasks (see stats)");
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

    if wigle_client.is_configured() {
        // Positive counterpart to the not-configured warning, so a successful
        // secrets load (incl. via systemd LoadCredential) is observable in the
        // journal rather than only its absence (task-0205 AC#3). Never logs the
        // credential values.
        info!("WiGLE credentials configured - remote lookups enabled");
    } else {
        tracing::warn!("WiGLE credentials not configured - remote lookups disabled");
    }

    let shutdown = tokio_util::sync::CancellationToken::new();
    let state = Arc::new(DaemonState {
        db: std::sync::Mutex::new(db),
        debouncer: tokio::sync::Mutex::new(debouncer),
        apple: apple::AppleClient::with_timeout(http_timeout),
        nominatim: nominatim::NominatimClient::with_timeout(nominatim_timeout),
        geocode: server::geocode::GeocodeScheduler::new(shutdown.clone()),
        address_cache: std::sync::Mutex::new(server::AddressCache::with_ttl_days(
            args.address_cache_ttl_days,
        )),
        args: args.clone(),
        wigle: wigle_client,
        last_fix: tokio::sync::Mutex::new(initial_last_fix),
        inflight: std::sync::Mutex::new(std::collections::HashSet::new()),
        db_write_failures: std::sync::atomic::AtomicU64::new(0),
        shutdown,
        tasks: std::sync::Mutex::new(tokio::task::JoinSet::new()),
        orphan_tasks_aborted: std::sync::atomic::AtomicU64::new(orphans_last_shutdown),
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

    let geocode_state = Arc::clone(&state);
    let geocode_handle = tokio::spawn(async move {
        server::geocode::run_worker(geocode_state).await;
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

    // Set up the mandatory control-channel unix listener BEFORE spawning the
    // server task. A failure here — a live socket already owns the path, or the
    // chmod could not be verified — must abort startup with a non-zero exit
    // rather than run degraded (task-0196). We handle stale socket files at
    // startup (not on run_server's exit, which is an abort at shutdown with no
    // chance to clean up); `cleanup_socket_path` is a best-effort unlink after
    // the drain for self-bound sockets (None when systemd owns the socket).
    let (unix_listener, cleanup_socket_path) =
        setup_unix_listener(&args).context("setting up the control-channel unix socket")?;

    let server_state = Arc::clone(&state);
    let mut server_handle =
        tokio::spawn(async move { server::run_server(server_state, unix_listener).await });

    // Wait for a shutdown signal or an unexpected server-task termination.
    // Both SIGINT (Ctrl-C) and SIGTERM (systemctl stop, docker stop, etc.) are
    // handled; their handlers were installed early (see above), so a signal
    // that arrived during startup is latched and handled here once startup
    // finishes. A server failure uses the SAME orderly drain, but is carried
    // through to the final Result so #[tokio::main] exits non-zero and
    // systemd Restart=on-failure can recover it (task-0241).
    let server_failure = tokio::select! {
        // If a signal and server failure become ready together, the failure
        // must win. An unbiased select could randomly misclassify a panicked
        // accept loop as a successful signal shutdown (exit 0).
        biased;
        res = &mut server_handle => {
            // This arm consumes the JoinHandle's output; it must NOT be
            // awaited again or tokio panics on the second poll (task-0195).
            let failure = server_exit_error(res);
            error!("{failure:#}");
            Some(failure)
        }
        _ = sigint.recv() => {
            info!("received SIGINT, shutting down");
            None
        }
        _ = sigterm.recv() => {
            info!("received SIGTERM, shutting down");
            None
        }
    };

    // task-0190: level-triggered cancel. Sticky, so a loop that is mid-
    // iteration (not parked in its select!) still observes it at the next
    // await instead of missing the wakeup and running forever.
    state.shutdown.cancel();

    // The server is special-cased for task-0195. On the failure path the
    // select arm already consumed its output, so touching it again would be a
    // second poll. On a signal, select only observed Pending: request abort of
    // the accept loop (it has no state to flush), then move the handle into the
    // SAME bounded drain as every other task. Tokio abort is cooperative, so
    // awaiting it here without the drain timeout would make SIGTERM unbounded.
    let server_to_drain = if server_failure.is_none() {
        server_handle.abort();
        Some(server_handle)
    } else {
        None
    };
    let named = vec![scan_handle, pending_handle, geocode_handle, history_handle];

    // Take the tracked-spawn JoinSet out of shared state so we can drive it to
    // completion here (a std::sync::Mutex guard cannot be held across await).
    let tracked = std::mem::take(&mut *state.tasks.lock().unwrap_or_else(|e| e.into_inner()));

    // task-0190/0102: wait up to the budget, then force-abort survivors.
    let aborted = run_shutdown_drain(Duration::from_secs(5), named, server_to_drain, tracked).await;
    if aborted > 0 {
        // Journal observability, independent of the stats persist below.
        info!("shutdown drain force-aborted {aborted} orphaned tasks");
        state
            .orphan_tasks_aborted
            .fetch_add(aborted, std::sync::atomic::Ordering::Relaxed);
    }
    // Persist THIS shutdown's count — 0 included, so a stale non-zero from an
    // older shutdown cannot masquerade as current. The next run loads it at
    // startup and serves it via stats; the in-memory bump above happens after
    // the server socket has closed and is observable by nobody (task-0102
    // review). A process-level crash/SIGKILL skips this write. A caught server
    // task failure reaches this orderly cleanup before main returns Err, so
    // the persisted value describes the last fully-drained shutdown even when
    // its process status is non-zero.
    {
        let db = server::lock_db(&state);
        if let Err(e) = db.set_metadata(ORPHANS_ABORTED_KEY, &aborted.to_string()) {
            tracing::warn!("failed to persist {ORPHANS_ABORTED_KEY}: {e}");
            state.record_db_failure();
        }
    }

    // Best-effort: remove the self-bound socket file so the next start does not
    // have to treat it as stale. Socket-activated sockets (cleanup path None)
    // are owned by systemd and left untouched (task-0196). A stale leftover is
    // harmless anyway — the next startup detects and clears it — so a failure
    // here is warn-only.
    if let Some(path) = &cleanup_socket_path {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(
                "failed to remove socket file {} at shutdown: {e}",
                path.display()
            ),
        }
    }

    match &server_failure {
        Some(_) => error!("whereamid stopped after unexpected server task failure"),
        None => info!("whereamid stopped"),
    }
    shutdown_result(server_failure)
}

/// Convert every way the server task can end into the fatal error propagated
/// by `main` after the orderly shutdown drain.
///
/// `run_server` is an accept loop and has no successful terminal state. Its
/// inner `Ok(())`, a returned error, and an outer Tokio `JoinError` (including
/// panic) are therefore all unexpected. Keeping this classification in a
/// small pure seam makes the process-status policy deterministic to test
/// without adding a production crash flag solely for an integration test.
fn server_exit_error(
    result: std::result::Result<Result<()>, tokio::task::JoinError>,
) -> anyhow::Error {
    match result {
        Ok(Ok(())) => anyhow::anyhow!("server task exited unexpectedly"),
        Ok(Err(error)) => error.context("server task failed"),
        Err(error) => anyhow::anyhow!("server task join error: {error}"),
    }
}

/// Final process result after every shutdown path has completed the same
/// cancellation, drain, metadata persistence, and socket cleanup.
/// `#[tokio::main]` maps `Err` to a non-zero process exit.
fn shutdown_result(server_failure: Option<anyhow::Error>) -> Result<()> {
    match server_failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Create every missing component of `dir` with mode 0700 (owner-only).
///
/// Uses `DirBuilder::mode` so the restrictive permission is applied at
/// creation time — there is no window where the directory briefly exists 0755
/// before a follow-up chmod. `mode()` applies ONLY to components this call
/// creates; pre-existing ancestors (e.g. `/var/lib`, `/home/<user>`, `/tmp`)
/// are left untouched, which is correct — the daemon must never re-permission
/// a shared parent it did not create. Unix-only by construction: whereamid is
/// Linux-only (tokio unix signals, nmcli/iw). task-0197.
fn ensure_private_dir(dir: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("creating state directory {}", dir.display()))
}

/// Warn (do NOT chmod) if `dir` is readable or traversable by group or other.
/// The daemon only tightens directories it created itself
/// ([`ensure_private_dir`]); a pre-existing state directory may be shared, so
/// we surface the exposure for the operator to fix rather than silently
/// changing a directory we do not own. task-0197.
fn warn_if_dir_accessible(dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(dir) {
        Ok(md) => {
            let mode = md.permissions().mode();
            if mode & 0o077 != 0 {
                tracing::warn!(
                    "state directory {} is group/other-accessible (mode {:o}); the \
                     location history in the database may be exposed to other local \
                     users — consider chmod 700",
                    dir.display(),
                    mode & 0o7777
                );
            }
        }
        Err(e) => {
            tracing::warn!("could not stat state directory {} ({e})", dir.display());
        }
    }
}

/// Resolve the effective unix control-socket path when the daemon self-binds.
/// Pure (env is passed in) so the resolution order is unit-testable. The
/// explicit `--socket` flag wins; otherwise `$XDG_RUNTIME_DIR/whereamid.sock`;
/// otherwise `/run/whereami/whereamid.sock`. task-0196.
fn resolve_socket_path(
    socket_flag: Option<&str>,
    xdg_runtime_dir: Option<&std::ffi::OsStr>,
) -> PathBuf {
    if let Some(s) = socket_flag {
        return PathBuf::from(s);
    }
    if let Some(dir) = xdg_runtime_dir {
        if !dir.is_empty() {
            return Path::new(dir).join("whereamid.sock");
        }
    }
    PathBuf::from("/run/whereami/whereamid.sock")
}

/// Build the control-channel unix listener, honoring systemd socket activation
/// (task-0196). Returns the tokio listener and, for a self-bound socket, the
/// path to unlink at shutdown (None when systemd owns the socket). Must run
/// inside the tokio runtime (`from_std` registers with the reactor).
fn setup_unix_listener(args: &Args) -> Result<(tokio::net::UnixListener, Option<PathBuf>)> {
    // systemd socket activation: an inherited fd wins over --socket. systemd
    // created the socket with the exact owner/group/mode from the .socket unit
    // and owns its lifecycle, so we skip stale-handling, chmod and cleanup.
    if let Some(fd) = server::sd_listen_fd()? {
        if args.socket.is_some() {
            warn!("--socket is ignored: systemd passed a socket-activated fd (the fd wins)");
        }
        info!("control channel: adopting systemd socket-activated fd {fd}");
        // Safety: sd_listen_fd verified LISTEN_PID == our pid and LISTEN_FDS ==
        // 1, so fd `SD_LISTEN_FDS_START` is addressed to this process and
        // nothing else in this program uses it — we take sole ownership here.
        // That the fd actually IS a listening AF_UNIX socket is NOT implied by
        // the env vars; verify_unix_listener_fd checks it explicitly below.
        let std_listener = unsafe { std::os::unix::net::UnixListener::from_raw_fd(fd) };
        server::verify_unix_listener_fd(&std_listener)?;
        // systemd passes activation fds without close-on-exec; set it so the
        // nmcli/iw child processes this daemon spawns cannot inherit the
        // control socket.
        server::set_cloexec(fd).context("setting FD_CLOEXEC on socket-activated fd")?;
        std_listener
            .set_nonblocking(true)
            .context("set_nonblocking on socket-activated fd")?;
        let listener = tokio::net::UnixListener::from_std(std_listener)
            .context("adopting socket-activated fd into tokio")?;
        return Ok((listener, None));
    }

    // Self-bind path. Compute the socket path and make sure its parent exists
    // and is owner-only (a no-op when it already exists, e.g. /run/user/<uid>).
    let path = resolve_socket_path(
        args.socket.as_deref(),
        std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
    );
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            ensure_private_dir(parent)
                .with_context(|| format!("preparing socket directory {}", parent.display()))?;
        }
    }
    info!("control channel: unix socket {}", path.display());
    let std_listener = server::self_bind_unix(&path)?;
    std_listener
        .set_nonblocking(true)
        .context("set_nonblocking on unix socket")?;
    let listener = tokio::net::UnixListener::from_std(std_listener)
        .context("adopting unix socket into tokio")?;
    Ok((listener, Some(path)))
}

/// Metadata key holding the number of tasks force-aborted at the end of the
/// last clean shutdown's drain. Persisted at shutdown and reloaded at startup
/// so `stats.orphan_tasks_aborted` reports the previous run's aborts — the
/// count is produced only after the server socket has closed, so within a
/// single run it is observable by nobody (task-0102 review).
const ORPHANS_ABORTED_KEY: &str = "orphans_aborted_last_shutdown";

/// Load the persisted forced-abort count from the last clean shutdown.
/// Missing or malformed values count as 0 — better to under-report a
/// diagnostic than to refuse to start.
fn load_orphan_count(db: &Database) -> u64 {
    match db.get_metadata(ORPHANS_ABORTED_KEY) {
        Ok(Some(v)) => v.parse().unwrap_or_else(|_| {
            tracing::warn!("corrupt {ORPHANS_ABORTED_KEY} metadata value {v:?}; treating as 0");
            0
        }),
        Ok(None) => 0,
        Err(e) => {
            tracing::warn!("failed to read {ORPHANS_ABORTED_KEY}: {e}");
            0
        }
    }
}

/// Drive the cooperative shutdown drain (task-0190/0102/0195).
///
/// The caller has already signalled `state.shutdown.cancel()`. This waits up
/// to `budget` for the `named` loop handles, the optional server handle, and
/// the `tracked` JoinSet to finish cooperatively; on timeout it force-aborts
/// everything still running and returns the number of tasks that had to be
/// aborted (named loops + server + tracked spawns), which the caller records
/// into `orphan_tasks_aborted`.
///
/// Every JoinHandle is polled at most once (task-0195): the graceful future
/// consumes the `named`/`server` handles, and the pre-extracted `AbortHandle`s
/// are the only thing touched on the timeout path — handles are never
/// re-awaited. The server is separate because its output is `Result<()>`,
/// unlike the background loops' `()`.
async fn run_shutdown_drain(
    budget: Duration,
    named: Vec<tokio::task::JoinHandle<()>>,
    server: Option<tokio::task::JoinHandle<Result<()>>>,
    mut tracked: tokio::task::JoinSet<()>,
) -> u64 {
    let named_aborts: Vec<tokio::task::AbortHandle> =
        named.iter().map(|h| h.abort_handle()).collect();
    let server_abort = server.as_ref().map(|h| h.abort_handle());

    // Graceful phase. `named` is moved in and consumed (owned `JoinHandle`s,
    // awaited once each); `tracked` is borrowed so it survives a timeout for
    // the force-abort + count below.
    let graceful = async {
        for h in named {
            let _ = h.await;
        }
        if let Some(h) = server {
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
    if let Some(a) = &server_abort {
        if !a.is_finished() {
            a.abort();
            aborted += 1;
        }
    }
    // Reap tracked tasks that completed on their own but were never joined —
    // the graceful phase drains `named` first, so a wedged named handle
    // starves the tracked reap and JoinSet::len() would overcount finished
    // tasks as aborted. After the reap, len() is genuinely-running survivors.
    while tracked.try_join_next().is_some() {}
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
                // Tracked so the shutdown drain waits for it (task-0102);
                // spawn_tracked also refuses once cancelled (task-0190 AC#2).
                // The is_cancelled() guard here just avoids the needless
                // Arc/Vec clone when we already know we are shutting down.
                if !stable.is_empty() && !state.shutdown.is_cancelled() {
                    let resolve_state = Arc::clone(&state);
                    let stable_vec: Vec<String> = stable.into_iter().collect();
                    state.spawn_tracked(async move {
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
mod server_exit_tests {
    //! task-0241: deterministic policy tests for unexpected server-task
    //! termination. The real accept loop is intentionally infinite and has no
    //! safe external input that makes it return or panic, so exercising the
    //! JoinHandle classification directly avoids a production fault-injection
    //! flag. `tests/shutdown.rs::sigterm_exits_within_drain_budget` remains the
    //! real-binary exit-0 control for the signal path.
    use super::{server_exit_error, shutdown_result};
    use anyhow::Result;

    #[test]
    fn unexpected_clean_server_completion_is_fatal() {
        let failure = server_exit_error(Ok(Ok(())));
        assert_eq!(failure.to_string(), "server task exited unexpectedly");
        assert!(
            shutdown_result(Some(failure)).is_err(),
            "unexpected server completion must become main's non-zero result"
        );
    }

    #[test]
    fn returned_server_error_is_fatal_and_preserved() {
        let failure = server_exit_error(Ok(Err(anyhow::anyhow!("accept loop failed"))));
        assert_eq!(
            format!("{failure:#}"),
            "server task failed: accept loop failed"
        );
        assert!(shutdown_result(Some(failure)).is_err());
    }

    #[tokio::test]
    async fn panicked_server_join_is_fatal() {
        let handle: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async {
            panic!("synthetic accept-loop panic");
        });
        let failure = server_exit_error(handle.await);
        let message = failure.to_string();
        assert!(
            message.contains("server task join error") && message.contains("panicked"),
            "panic JoinError must remain diagnostic: {message}"
        );
        assert!(shutdown_result(Some(failure)).is_err());
    }

    #[test]
    fn signal_shutdown_remains_successful() {
        assert!(
            shutdown_result(None).is_ok(),
            "SIGINT/SIGTERM path must remain exit 0"
        );
    }
}

#[cfg(test)]
mod shutdown_drain_tests {
    //! Shutdown-drain tests for task-0190/0102/0195/0241. The ordinary async
    //! cases use a paused clock, which auto-advances to the budget timeout.
    //! The non-cooperative server regression deliberately uses real-time
    //! `spawn_blocking` work (which paused Tokio time cannot model) and carries
    //! an independent outer watchdog so a regression fails instead of hanging.
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
        let aborted = run_shutdown_drain(Duration::from_secs(5), named, None, tracked).await;
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
        let aborted = run_shutdown_drain(Duration::from_secs(1), Vec::new(), None, tracked).await;
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
        let aborted = run_shutdown_drain(Duration::from_secs(1), named, None, JoinSet::new()).await;
        assert_eq!(aborted, 1);
    }

    /// task-0241 review regression: abort is cooperative. A server task in
    /// non-yielding/blocking work cannot be awaited outside the shutdown
    /// budget, or SIGTERM may hang forever before metadata/socket cleanup.
    /// `spawn_blocking` deliberately models work that has already started and
    /// therefore ignores `JoinHandle::abort`; the bounded drain must time out,
    /// count it, and return while the blocking closure is still held.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drain_bounds_non_cooperative_server_handle() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let server = tokio::task::spawn_blocking(move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok::<(), anyhow::Error>(())
        });

        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("blocking server task must start");
        server.abort(); // Ineffective after spawn_blocking has started.

        let start = Instant::now();
        let outcome = tokio::time::timeout(
            Duration::from_secs(2),
            run_shutdown_drain(
                Duration::from_millis(50),
                Vec::new(),
                Some(server),
                JoinSet::new(),
            ),
        )
        .await;
        let elapsed = start.elapsed();

        // Release before unwrapping/asserting so a timeout or failed assertion
        // cannot strand a blocking task during Tokio runtime teardown.
        release_tx.send(()).unwrap();
        let aborted = outcome.expect("bounded server drain exceeded its 2s test watchdog");
        assert_eq!(aborted, 1, "the stuck server handle must be counted");
        assert!(
            elapsed < Duration::from_secs(1),
            "server drain exceeded its 50ms budget: {elapsed:?}"
        );
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
        let aborted = run_shutdown_drain(Duration::from_secs(1), Vec::new(), None, tracked).await;
        assert_eq!(
            aborted, 1,
            "only the still-running task counts; the finished one was reaped"
        );
    }

    /// Review C1 regression: a wedged NAMED handle starves the graceful
    /// phase's tracked reap (`named` drains first), so tracked tasks that
    /// completed on their own would be miscounted as aborted without the
    /// timeout-path `try_join_next` reap. Expected: wedged named (1) + stuck
    /// tracked (1) = 2, not 4.
    #[tokio::test(start_paused = true)]
    async fn drain_does_not_count_finished_tracked_when_named_wedges() {
        let named = vec![tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        })];
        let mut tracked = JoinSet::new();
        tracked.spawn(async {}); // completes on its own, never joined gracefully
        tracked.spawn(async {}); // completes on its own, never joined gracefully
        tracked.spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }); // genuine survivor
        let aborted = run_shutdown_drain(Duration::from_secs(1), named, None, tracked).await;
        assert_eq!(
            aborted, 2,
            "finished-but-unjoined tracked tasks must be reaped before counting"
        );
    }
}

#[cfg(test)]
mod orphan_count_tests {
    //! Persist/reload cycle for `orphan_tasks_aborted` (task-0102 review):
    //! the count is written to DB metadata at clean shutdown and rehydrated
    //! at startup, because it is produced only after the server socket has
    //! closed — the previous run's value is the only observable one.
    use super::{load_orphan_count, ORPHANS_ABORTED_KEY};
    use crate::db::Database;

    #[test]
    fn missing_key_loads_zero() {
        let db = Database::open_memory().unwrap();
        assert_eq!(load_orphan_count(&db), 0);
    }

    #[test]
    fn persisted_count_round_trips() {
        let db = Database::open_memory().unwrap();
        db.set_metadata(ORPHANS_ABORTED_KEY, "7").unwrap();
        assert_eq!(load_orphan_count(&db), 7);
        // A later clean shutdown overwrites — including with 0, so a stale
        // non-zero cannot masquerade as the most recent shutdown's result.
        db.set_metadata(ORPHANS_ABORTED_KEY, "0").unwrap();
        assert_eq!(load_orphan_count(&db), 0);
    }

    #[test]
    fn malformed_value_loads_zero() {
        let db = Database::open_memory().unwrap();
        db.set_metadata(ORPHANS_ABORTED_KEY, "not-a-number")
            .unwrap();
        assert_eq!(load_orphan_count(&db), 0);
    }
}

#[cfg(test)]
mod state_dir_tests {
    //! task-0197: the daemon must create its state directory owner-only
    //! (0700), including every nested component it makes, so the location
    //! history it stores is not exposed to other local users on manual /
    //! --db-relocated runs.
    use super::ensure_private_dir;
    use std::os::unix::fs::PermissionsExt;

    fn unique_tempdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "whereamid-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mode_bits(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// AC#2 (dir half): ensure_private_dir creates ALL missing nested
    /// components at 0700, not just the leaf. The pre-existing base tempdir
    /// (0755, the analog of /var/lib) is left untouched — only the components
    /// we create get the tight mode.
    #[test]
    fn ensure_private_dir_creates_nested_components_0700() {
        let base = unique_tempdir("statedir");
        // Capture the pre-existing base mode; it must be unchanged afterward
        // (robust to whatever umask create_dir_all ran under).
        let base_mode_before = mode_bits(&base);
        let nested = base.join("a").join("b").join("whereami");
        ensure_private_dir(&nested).unwrap();

        assert_eq!(
            mode_bits(&base.join("a")),
            0o700,
            "created 'a' must be 0700"
        );
        assert_eq!(
            mode_bits(&base.join("a").join("b")),
            0o700,
            "created 'b' must be 0700"
        );
        assert_eq!(mode_bits(&nested), 0o700, "leaf must be 0700");
        // We must not have re-permissioned the pre-existing base directory.
        assert_eq!(
            mode_bits(&base),
            base_mode_before,
            "pre-existing base dir must be left untouched"
        );

        // Idempotent: a second call on the now-existing path is a no-op that
        // neither errors nor loosens the mode.
        ensure_private_dir(&nested).unwrap();
        assert_eq!(mode_bits(&nested), 0o700);

        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod socket_path_tests {
    //! task-0196: unix control-socket path resolution order (pure; env passed
    //! in, so it never reads the host's real XDG_RUNTIME_DIR).
    use super::resolve_socket_path;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    #[test]
    fn explicit_socket_flag_wins_over_xdg() {
        assert_eq!(
            resolve_socket_path(Some("/tmp/custom.sock"), Some(OsStr::new("/run/user/1000"))),
            PathBuf::from("/tmp/custom.sock")
        );
    }

    #[test]
    fn xdg_runtime_dir_used_when_no_flag() {
        assert_eq!(
            resolve_socket_path(None, Some(OsStr::new("/run/user/1000"))),
            PathBuf::from("/run/user/1000/whereamid.sock")
        );
    }

    #[test]
    fn falls_back_to_run_when_no_xdg() {
        assert_eq!(
            resolve_socket_path(None, None),
            PathBuf::from("/run/whereami/whereamid.sock")
        );
        // An empty XDG_RUNTIME_DIR is treated as unset.
        assert_eq!(
            resolve_socket_path(None, Some(OsStr::new(""))),
            PathBuf::from("/run/whereami/whereamid.sock")
        );
    }
}
