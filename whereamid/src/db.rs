//! SQLite database: schema creation, WAL mode, CRUD operations for aps/not_found/pending/metadata.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;
use tracing::{debug, info};

use crate::quality::{Confidence, ConfidenceDb, FixQuality};

/// Information about a resolved access point.
#[derive(Debug, Clone)]
pub struct ApInfo {
    pub bssid: String,
    pub ssid: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub encryption: Option<String>,
    /// Wi-Fi frequency in MHz — the CANONICAL physical identifier. The old
    /// band-less `channel` column was redundant (a pure function of frequency,
    /// and ambiguous once 6 GHz reuses low channel numbers) and was dropped at
    /// schema v9 (task-0123). Consumers that need a channel derive it on the fly
    /// via `whereami_protocol::Channel::from_frequency(frequency)`.
    pub frequency: Option<i32>,
    pub city: Option<String>,
    pub country: Option<String>,
    /// Whether this AP's most recently persisted scanner SSID matched the
    /// mobile-SSID denylist. The value is recomputed on every upsert using
    /// the daemon's current denylist. Existing rows are deliberately NOT
    /// retroactively re-evaluated when that denylist changes; they update only
    /// after a later successful resolution reaches this AP's upsert path.
    pub is_mobile: bool,
    /// Provenance of the position. Internally a typed enum; the SQLite
    /// boundary stringifies via `Source::as_str` and parses back via
    /// `Source::from_db_str`. No string literals appear in production
    /// callers.
    pub source: Source,
}

// task-0055: Source enum extracted to db/source.rs. Public API
// re-exported here so existing call sites (`crate::db::Source`) keep
// working unchanged.
mod source;
pub use source::Source;

/// Pending lookup work. The scanner-observed SSID must survive an offline
/// provider pass so the eventual cache upsert classifies the same observation,
/// rather than Apple/WiGLE metadata (task-0093).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAp {
    pub bssid: String,
    pub ssid: Option<String>,
    pub attempts: i32,
}

pub(crate) struct Database {
    conn: Connection,
    /// Immutable for one daemon process, but supplied to every writer actor
    /// from that process's current CLI configuration. Keeping it at the DB
    /// boundary makes every upsert use the same authoritative classifier.
    mobile_ssid_denylist: Vec<String>,
}

/// How long a database actor waits for another SQLite connection to release a
/// lock.  This is deliberately shorter than the daemon's five-second shutdown
/// drain, so an external writer cannot strand shutdown indefinitely.
pub const DB_BUSY_TIMEOUT: Duration = Duration::from_secs(2);

const PRUNE_FIXES_CHUNK_SQL: &str = "DELETE FROM fixes WHERE id IN (
         SELECT id FROM fixes INDEXED BY idx_fixes_at
         WHERE at_rfc3339 < ?1 ORDER BY at_rfc3339 LIMIT ?2
     )";

const COUNT_FIXES_IN_RANGE_SQL: &str = "SELECT COUNT(*) FROM fixes INDEXED BY idx_fixes_at
     WHERE at_rfc3339 >= ?1 AND at_rfc3339 <= ?2";

const SELECT_NEWEST_FIXES_IN_RANGE_SQL: &str =
    "SELECT id, at_rfc3339, lat, lon, accuracy_m, n_sources,
            implied_speed_kmh, last_fix_age_secs, inlier_ratio, confidence, suspect_reason
     FROM fixes INDEXED BY idx_fixes_at
     WHERE at_rfc3339 >= ?1 AND at_rfc3339 <= ?2
     ORDER BY at_rfc3339 DESC, id DESC
     LIMIT ?3";

const PRUNE_APS_CHUNK_SQL: &str = "DELETE FROM aps WHERE bssid IN (
         SELECT bssid FROM aps INDEXED BY idx_aps_last_seen
         WHERE last_seen < ?1 ORDER BY last_seen LIMIT ?2
     )";

/// Operational ceiling shared by daemon validation and the Nix module. It is
/// intentionally generous (one thousand years) while keeping cutoff arithmetic
/// representable and rejecting absurd values before startup side effects.
pub const MAX_RETENTION_DAYS: i64 = 365_000;

pub const RETENTION_CHUNK_SIZE: usize = 1_000;

type DbJob = Box<dyn FnOnce(&mut Database) + Send + 'static>;

const FAIL_STOP_EXIT_CODE: i32 = 70;

#[cfg(test)]
pub(crate) const FAIL_STOP_TEST_ENTERED_ENV: &str = "WHEREAMID_FAIL_STOP_TEST_ENTERED";
#[cfg(test)]
pub(crate) const FAIL_STOP_TEST_RELEASE_ENV: &str = "WHEREAMID_FAIL_STOP_TEST_RELEASE";

fn fail_stop_after_actor_panic(actor: &'static str, operation: &'static str) -> ! {
    #[cfg(test)]
    if let (Some(entered), Some(release)) = (
        std::env::var_os(FAIL_STOP_TEST_ENTERED_ENV),
        std::env::var_os(FAIL_STOP_TEST_RELEASE_ENV),
    ) {
        // Exact subprocess-only fatal-path barrier. The guarded caller invokes
        // fail-stop from its inner catch while its invariant guard is still a
        // live local, so pausing here must keep privacy publication barred.
        if std::fs::write(&entered, b"entered").is_ok() {
            let release = std::path::PathBuf::from(release);
            while !release.exists() {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    // Diagnostics are best-effort, but a broken subscriber/stderr must never
    // turn fail-stop back into actor-only failure. Catch any secondary panic,
    // then terminate through the non-returning process primitive. The guarded
    // call site still owns its publication barrier throughout this function,
    // including if either diagnostic blocks.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tracing::error!(
            actor,
            operation,
            exit_code = FAIL_STOP_EXIT_CODE,
            "fatal database invariant transition panicked; terminating process"
        );
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr().lock(),
            "fatal: database actor {actor} fail-stop operation \
             {operation:?} panicked; exiting {FAIL_STOP_EXIT_CODE}"
        );
    }));
    std::process::exit(FAIL_STOP_EXIT_CODE)
}

enum DbCommand {
    Run(DbJob),
    /// This job owns an already-committed invariant transition whose unwind
    /// cannot be repaired by dropping its result. A panic must terminate the
    /// whole process before it can serve split persisted/in-memory state.
    RunFailStop {
        operation: &'static str,
        job: DbJob,
    },
    Stop(tokio::sync::oneshot::Sender<()>),
}

/// Marks an enqueued request abandoned when its async waiter is dropped. The
/// actor checks the flag before starting queued work; reader progress handlers
/// also check it while SQLite is executing a long query.
struct CancelOnDrop {
    cancelled: Arc<AtomicBool>,
    armed: bool,
}

impl CancelOnDrop {
    fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.cancelled.store(true, Ordering::Release);
        }
    }
}

/// Owned WiGLE quota reservation. It is created before the reservation command
/// is enqueued, so cancellation at every await boundary has an owner that can
/// append a conditional refund to the same writer FIFO.
// The package's library target intentionally omits the daemon provider module
// but compiles this shared db module, so the lease is reachable only in the
// binary target there.
#[allow(dead_code)]
pub(crate) struct QuotaLease {
    writer: mpsc::Sender<DbCommand>,
    committed: Arc<AtomicBool>,
    failures: Arc<std::sync::atomic::AtomicU64>,
    active: bool,
}

/// Result of the synchronous, transactional quota decision. Corruption is a
/// policy decision rather than a SQLite failure: it denies the reservation
/// without changing the persisted row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuotaDecision {
    Reserved,
    Exhausted,
    CounterCorrupt,
}

/// Actor-facing quota decision. A successful reservation transfers the
/// cancellation-owned lease; the two denial cases carry no lease.
#[allow(dead_code)]
pub(crate) enum QuotaReservation {
    Reserved(QuotaLease),
    Exhausted,
    CounterCorrupt,
}

#[allow(dead_code)]
impl QuotaLease {
    fn new(
        writer: mpsc::Sender<DbCommand>,
        committed: Arc<AtomicBool>,
        failures: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        Self {
            writer,
            committed,
            failures,
            active: true,
        }
    }

    /// The upstream request consumed the reservation (Found/NotFound).
    pub(crate) fn consume(mut self) {
        self.active = false;
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for QuotaLease {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let committed = Arc::clone(&self.committed);
        let committed_on_send = Arc::clone(&self.committed);
        let failures = Arc::clone(&self.failures);
        let failures_on_send = Arc::clone(&failures);
        let command = DbCommand::Run(Box::new(move |db| {
            // FIFO ordering is the protocol: this runs after the matching
            // reservation command, whether that command was still queued,
            // actively committing, or had delivered its result. swap(false)
            // makes the conditional refund exactly-once.
            if committed.swap(false, Ordering::AcqRel) {
                if let Err(error) = db.refund_wigle_lookup() {
                    // Counter corruption already emitted the more accurate,
                    // escaped refund-context warning at the parse boundary.
                    if error.downcast_ref::<CorruptWigleLookupsValue>().is_none() {
                        tracing::warn!("failed to refund WiGLE quota lease on drop: {error}");
                    }
                    failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }));
        if self.writer.send(command).is_err() && committed_on_send.load(Ordering::Acquire) {
            tracing::warn!("failed to enqueue WiGLE quota refund: database actor stopped");
            failures_on_send.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[derive(Default)]
struct ReservationTestHooks {
    queued: Option<tokio::sync::oneshot::Sender<()>>,
    committed: Option<tokio::sync::oneshot::Sender<()>>,
    release_after_commit: Option<mpsc::Receiver<()>>,
    decision: Option<tokio::sync::oneshot::Sender<QuotaDecision>>,
    release_after_decision: Option<mpsc::Receiver<()>>,
}

struct DbActor {
    tx: mpsc::Sender<DbCommand>,
    thread: Option<JoinHandle<()>>,
}

impl DbActor {
    fn spawn(
        name: &'static str,
        open: impl FnOnce() -> Result<Database> + Send + 'static,
    ) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<DbCommand>();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || match open() {
                Ok(mut db) => {
                    let _ = ready_tx.send(Ok(()));
                    let mut stop_done = None;
                    while let Ok(command) = rx.recv() {
                        match command {
                            DbCommand::Run(job) => {
                                // A buggy command must fail its caller, not permanently kill
                                // the database service and every later request.
                                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    job(&mut db)
                                }))
                                .is_err()
                                {
                                    tracing::error!(actor = name, "database actor job panicked");
                                }
                            }
                            DbCommand::RunFailStop { operation, job } => {
                                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    job(&mut db)
                                }))
                                .is_err()
                                {
                                    // Process-wide termination is intentional: unwinding only this
                                    // actor would let the daemon continue with irreconcilable state.
                                    fail_stop_after_actor_panic(name, operation);
                                }
                            }
                            DbCommand::Stop(done) => {
                                stop_done = Some(done);
                                break;
                            }
                        }
                    }
                    // Connection::drop may checkpoint/close WAL resources. Do
                    // it on the actor thread before acknowledging shutdown so
                    // the Tokio caller's subsequent JoinHandle::join is truly
                    // non-blocking and performs no SQLite work.
                    drop(db);
                    if let Some(done) = stop_done {
                        let _ = done.send(());
                    }
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            })
            .with_context(|| format!("spawning {name} database actor"))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                tx,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                anyhow::bail!("{name} database actor exited before startup completed")
            }
        }
    }

    fn join_thread(&mut self, errors: &mut Vec<String>) {
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                errors.push("database actor thread panicked during shutdown".to_owned());
            }
        }
    }

    fn finish_stop(errors: Vec<String>) -> Result<()> {
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(errors.join("; "))
        }
    }

    fn stop_blocking(mut self) -> Result<()> {
        self.stop_blocking_inner(None)
    }

    fn stop_blocking_with_signal(
        mut self,
        stop_queued: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<()> {
        self.stop_blocking_inner(stop_queued)
    }

    fn stop_blocking_inner(
        &mut self,
        stop_queued: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<()> {
        if self.thread.is_none() {
            return Ok(());
        }
        let (done_tx, mut done_rx) = tokio::sync::oneshot::channel();
        let mut errors = Vec::new();
        let stop_sent = self.tx.send(DbCommand::Stop(done_tx)).is_ok();
        if !stop_sent {
            errors.push("database actor stopped before shutdown request".to_owned());
        } else if let Some(stop_queued) = stop_queued {
            // This signal is deliberately emitted after the Stop command is
            // in the actor FIFO. It is used by the cancellation regression to
            // prove the blocking cleanup task owns the actor before aborting
            // the async waiter.
            let _ = stop_queued.send(());
        }
        self.join_thread(&mut errors);
        // Join proves the actor can no longer mutate the receiver. Inspect the
        // acknowledgement without Tokio's blocking_recv (which panics when a
        // fallback Drop happens on a runtime worker).
        if stop_sent && done_rx.try_recv().is_err() {
            errors.push("database actor stopped without acknowledging shutdown".to_owned());
        }
        Self::finish_stop(errors)
    }
}

impl Drop for DbActor {
    fn drop(&mut self) {
        if self.thread.is_none() {
            return;
        }
        // Exceptional ownership fallback for a fallible startup path that
        // returns after an actor has spawned. Normal DbRuntime shutdown calls
        // stop_blocking_inner first, takes the JoinHandle, and reaches this
        // Drop with no thread left to stop a second time.
        if let Err(error) = self.stop_blocking_inner(None) {
            tracing::error!("fallback database actor shutdown failed: {error:#}");
        }
    }
}

/// Cloneable, asynchronous access to the database. Every closure that can
/// touch rusqlite is constructed in this module and runs on one of the two
/// named OS threads; no connection, statement, row, or transaction escapes.
#[derive(Clone)]
pub struct DbHandle {
    writer: mpsc::Sender<DbCommand>,
    reader: mpsc::Sender<DbCommand>,
}

/// Owns the two database threads. Main keeps this owner alive until all
/// handlers/background tasks and final metadata writes have drained, then
/// performs an explicit FIFO shutdown.
pub struct DbRuntime {
    writer: Option<DbActor>,
    reader: Option<DbActor>,
}

#[derive(Debug, Default)]
pub struct DbStats {
    pub cached_aps: i64,
    pub pending_aps: i64,
    pub not_found_aps: i64,
    pub db_size_bytes: i64,
    pub wigle_lookups_today: u32,
}

/// Exact row counts removed by one committed privacy wipe transaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrivacyWipeCounts {
    pub aps_deleted: usize,
    pub pending_deleted: usize,
    pub not_found_deleted: usize,
    pub fixes_deleted: usize,
    pub last_fix_deleted: usize,
}

impl DbRuntime {
    /// Open/migrate the writer, then open the independent WAL reader. Startup
    /// coordination itself waits on Tokio's blocking pool; SQLite remains
    /// confined to the two dedicated actors.
    #[allow(dead_code)]
    pub async fn open(path: &Path) -> Result<(Self, DbHandle)> {
        Self::open_with_mobile_ssid_denylist(path, crate::scanner::default_mobile_ssid_denylist())
            .await
    }

    /// Open the database actors with the process's configured mobile-SSID
    /// denylist. The writer owns this value so all current and future upsert
    /// paths classify at one persistence boundary.
    pub async fn open_with_mobile_ssid_denylist(
        path: &Path,
        mobile_ssid_denylist: Vec<String>,
    ) -> Result<(Self, DbHandle)> {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || Self::open_blocking(path, mobile_ssid_denylist))
            .await
            .context("joining database startup coordinator")?
    }

    fn open_blocking(path: PathBuf, mobile_ssid_denylist: Vec<String>) -> Result<(Self, DbHandle)> {
        Self::open_blocking_with_reader(
            path,
            move |writer_path| {
                Database::open_with_mobile_ssid_denylist(&writer_path, mobile_ssid_denylist)
            },
            |reader_path| Database::open_reader(&reader_path),
        )
    }

    fn open_blocking_with_reader<W, R>(
        path: PathBuf,
        open_writer: W,
        open_reader: R,
    ) -> Result<(Self, DbHandle)>
    where
        W: FnOnce(PathBuf) -> Result<Database> + Send + 'static,
        R: FnOnce(PathBuf) -> Result<Database> + Send + 'static,
    {
        let writer_path = path.clone();
        let writer = DbActor::spawn("whereamid-db-writer", move || open_writer(writer_path))?;
        let reader = match DbActor::spawn("whereamid-db-reader", move || open_reader(path)) {
            Ok(reader) => reader,
            Err(error) => {
                return match writer.stop_blocking() {
                    Ok(()) => Err(error.context("opening database reader")),
                    Err(cleanup) => Err(anyhow::anyhow!(
                        "opening database reader failed: {error:#}; writer cleanup also failed: {cleanup:#}"
                    )),
                };
            }
        };
        let handle = DbHandle {
            writer: writer.tx.clone(),
            reader: reader.tx.clone(),
        };
        Ok((
            Self {
                writer: Some(writer),
                reader: Some(reader),
            },
            handle,
        ))
    }

    /// Shut down after all `DbHandle` owners have stopped enqueueing work.
    /// Both actors are first transferred to one blocking cleanup task, so
    /// cancelling the async waiter cannot detach either actor. Joining the OS
    /// threads therefore never blocks a Tokio worker during normal shutdown.
    pub async fn shutdown(mut self) -> Result<()> {
        let cleanup = self.schedule_shutdown(None, None);
        cleanup
            .await
            .context("joining database shutdown coordinator")?
    }

    fn schedule_shutdown(
        &mut self,
        writer_stop_queued: Option<tokio::sync::oneshot::Sender<()>>,
        cleanup_complete: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> tokio::task::JoinHandle<Result<()>> {
        let writer = self.writer.take().expect("DbRuntime writer already taken");
        let reader = self.reader.take().expect("DbRuntime reader already taken");

        // There must be no await between taking either actor and scheduling
        // this closure. Dropping the returned JoinHandle detaches the blocking
        // task but does not cancel it, so it retains both actors through join.
        tokio::task::spawn_blocking(move || {
            let writer = writer.stop_blocking_with_signal(writer_stop_queued);
            let reader = reader.stop_blocking();
            let result = Self::combine_shutdown_results(writer, reader);
            if let Some(cleanup_complete) = cleanup_complete {
                // Both stop_blocking calls have joined their actor threads.
                let _ = cleanup_complete.send(());
            }
            result
        })
    }

    fn combine_shutdown_results(writer: Result<()>, reader: Result<()>) -> Result<()> {
        match (writer, reader) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(writer), Err(reader)) => Err(anyhow::anyhow!(
                "writer shutdown failed: {writer:#}; reader shutdown failed: {reader:#}"
            )),
        }
    }

    #[cfg(test)]
    async fn shutdown_with_signals(
        mut self,
        writer_stop_queued: tokio::sync::oneshot::Sender<()>,
        cleanup_complete: tokio::sync::oneshot::Sender<()>,
    ) -> Result<()> {
        let cleanup = self.schedule_shutdown(Some(writer_stop_queued), Some(cleanup_complete));
        cleanup
            .await
            .context("joining database shutdown coordinator")?
    }

    #[cfg(test)]
    pub(crate) fn open_test() -> Result<(Self, DbHandle, Database)> {
        use rusqlite::OpenFlags;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let uri = format!(
            "file:whereamid-test-{}?mode=memory&cache=shared",
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        let writer_uri = uri.clone();
        let writer = DbActor::spawn("whereamid-test-db-writer", move || {
            Database::open_shared_memory(&writer_uri, true)
        })?;
        let reader_uri = uri.clone();
        let reader = DbActor::spawn("whereamid-test-db-reader", move || {
            Database::open_shared_memory(&reader_uri, false)
        })?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_URI;
        let conn = Connection::open_with_flags(&uri, flags)?;
        conn.busy_timeout(DB_BUSY_TIMEOUT)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let legacy = Database {
            conn,
            mobile_ssid_denylist: crate::scanner::default_mobile_ssid_denylist(),
        };
        let handle = DbHandle {
            writer: writer.tx.clone(),
            reader: reader.tx.clone(),
        };
        Ok((
            Self {
                writer: Some(writer),
                reader: Some(reader),
            },
            handle,
            legacy,
        ))
    }
}

impl Drop for DbRuntime {
    fn drop(&mut self) {
        // Exceptional fallback for owners that never reach explicit shutdown.
        // This may synchronously wait for active actor work, including when
        // dropped on a Tokio worker. Normal callers should use `shutdown`,
        // which transfers both actors to the blocking pool before awaiting.
        let mut errors = Vec::new();
        if let Some(writer) = self.writer.take() {
            if let Err(error) = writer.stop_blocking() {
                errors.push(format!("writer: {error:#}"));
            }
        }
        if let Some(reader) = self.reader.take() {
            if let Err(error) = reader.stop_blocking() {
                errors.push(format!("reader: {error:#}"));
            }
        }
        if !errors.is_empty() {
            tracing::error!(
                "fallback database actor shutdown failed: {}",
                errors.join("; ")
            );
        }
    }
}

impl DbHandle {
    async fn call_cancellable<R, F>(
        sender: &mpsc::Sender<DbCommand>,
        operation: F,
        queued: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Database) -> Result<R> + Send + 'static,
    {
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut cancel_on_drop = CancelOnDrop::new(Arc::clone(&cancelled));
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        sender
            .send(DbCommand::Run(Box::new(move |db| {
                if cancelled.load(Ordering::Acquire) {
                    return;
                }
                let _ = result_tx.send(operation(db));
            })))
            .map_err(|_| anyhow::anyhow!("database actor is not running"))?;
        if let Some(queued) = queued {
            let _ = queued.send(());
        }
        let result = result_rx
            .await
            .map_err(|_| anyhow::anyhow!("database actor dropped request without a result"))?;
        cancel_on_drop.disarm();
        result
    }

    async fn call_durable<R, F>(sender: &mpsc::Sender<DbCommand>, operation: F) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Database) -> Result<R> + Send + 'static,
    {
        Self::call_durable_queued(sender, operation, None).await
    }

    async fn call_durable_queued<R, F>(
        sender: &mpsc::Sender<DbCommand>,
        operation: F,
        queued: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Database) -> Result<R> + Send + 'static,
    {
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        sender
            .send(DbCommand::Run(Box::new(move |db| {
                let _ = result_tx.send(operation(db));
            })))
            .map_err(|_| anyhow::anyhow!("database actor is not running"))?;
        if let Some(queued) = queued {
            let _ = queued.send(());
        }
        result_rx
            .await
            .map_err(|_| anyhow::anyhow!("database actor dropped durable request"))?
    }

    /// Durable command for a transition that becomes irrecoverable part-way
    /// through its closure. `guard` remains a local outside the inner unwind
    /// boundary, so fail-stop begins before the invariant guard can drop.
    /// Normal `Result::Err` values release the guard and return to the caller;
    /// only a panic invokes the process-wide fail-stop policy.
    async fn call_durable_fail_stop_guarded_queued<R, G, F>(
        sender: &mpsc::Sender<DbCommand>,
        operation_name: &'static str,
        guard: G,
        operation: F,
        queued: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<(R, G)>
    where
        R: Send + 'static,
        G: Send + 'static,
        F: FnOnce(&mut Database, &mut G) -> Result<R> + Send + 'static,
    {
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        sender
            .send(DbCommand::RunFailStop {
                operation: operation_name,
                job: Box::new(move |db| {
                    let mut guard = guard;
                    let transition = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        operation(db, &mut guard)
                    }));
                    let result = match transition {
                        Ok(Ok(value)) => Ok((value, guard)),
                        Ok(Err(error)) => {
                            // The transaction rolled back and managed state was
                            // not touched. Release the actor-owned invariant
                            // guard before publishing the error, so response
                            // delivery cannot race its admission/lease drop.
                            drop(guard);
                            Err(error)
                        }
                        Err(_) => fail_stop_after_actor_panic("database writer", operation_name),
                    };
                    let _ = result_tx.send(result);
                }),
            })
            .map_err(|_| anyhow::anyhow!("database actor is not running"))?;
        if let Some(queued) = queued {
            let _ = queued.send(());
        }
        result_rx
            .await
            .map_err(|_| anyhow::anyhow!("database actor dropped fail-stop request"))?
    }

    async fn call_interruptible<R, F>(
        sender: &mpsc::Sender<DbCommand>,
        operation: F,
        queued: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Database) -> Result<R> + Send + 'static,
    {
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut cancel_on_drop = CancelOnDrop::new(Arc::clone(&cancelled));
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        sender
            .send(DbCommand::Run(Box::new(move |db| {
                if cancelled.load(Ordering::Acquire) {
                    return;
                }
                let progress_cancelled = Arc::clone(&cancelled);
                db.conn.progress_handler(
                    1_000,
                    Some(move || progress_cancelled.load(Ordering::Acquire)),
                );
                // Always clear the connection-local callback, including when a
                // buggy operation panics. The outer actor boundary logs the
                // resumed panic and remains reusable.
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(db)));
                db.conn.progress_handler(0, None::<fn() -> bool>);
                match outcome {
                    Ok(result) => {
                        let _ = result_tx.send(result);
                    }
                    Err(panic) => std::panic::resume_unwind(panic),
                }
            })))
            .map_err(|_| anyhow::anyhow!("database actor is not running"))?;
        if let Some(queued) = queued {
            let _ = queued.send(());
        }
        let result = result_rx
            .await
            .map_err(|_| anyhow::anyhow!("database actor dropped read request"))?;
        cancel_on_drop.disarm();
        result
    }

    async fn write<R, F>(&self, operation: F) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Database) -> Result<R> + Send + 'static,
    {
        Self::call_cancellable(&self.writer, operation, None).await
    }

    async fn write_durable<R, F>(&self, operation: F) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Database) -> Result<R> + Send + 'static,
    {
        Self::call_durable(&self.writer, operation).await
    }

    async fn read<R, F>(&self, operation: F) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Database) -> Result<R> + Send + 'static,
    {
        Self::call_interruptible(&self.reader, operation, None).await
    }

    pub async fn lookup_cached_touch(&self, bssids: Vec<String>) -> Result<Vec<ApInfo>> {
        self.write(move |db| db.lookup_cached_touch(&bssids)).await
    }

    /// Read cached APs in input order without refreshing `last_seen`.
    /// Autonomous locate uses this so every skip outcome is write-free.
    pub async fn lookup_cached_readonly(&self, bssids: Vec<String>) -> Result<Vec<ApInfo>> {
        self.read(move |db| db.lookup_cached_readonly(&bssids))
            .await
    }

    pub async fn cache_resolved_ap(&self, ap: ApInfo, delete_pending: bool) -> Result<()> {
        self.write(move |db| db.cache_resolved_ap(&ap, delete_pending))
            .await
    }

    pub async fn is_not_found(&self, bssid: impl Into<String>, ttl_days: i64) -> Result<bool> {
        let bssid = bssid.into();
        self.read(move |db| db.is_not_found(&bssid, ttl_days)).await
    }

    pub async fn record_not_found(
        &self,
        bssid: impl Into<String>,
        delete_pending: bool,
    ) -> Result<()> {
        let bssid = bssid.into();
        self.write(move |db| db.record_not_found(&bssid, delete_pending))
            .await
    }

    pub async fn insert_pending_many(
        &self,
        observations: Vec<(String, Option<String>)>,
    ) -> Result<()> {
        self.write(move |db| db.insert_pending_many(&observations))
            .await
    }

    pub async fn get_pending(&self, limit: usize) -> Result<Vec<PendingAp>> {
        self.read(move |db| db.get_pending(limit)).await
    }

    pub async fn increment_pending_attempts(&self, bssids: Vec<String>) -> Result<()> {
        self.write(move |db| {
            let tx = db
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            for bssid in bssids {
                tx.execute(
                    "UPDATE pending SET attempts = attempts + 1 WHERE bssid = ?1",
                    params![bssid],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn delete_expired_pending(&self, max_attempts: i32) -> Result<usize> {
        self.write(move |db| db.delete_expired_pending(max_attempts))
            .await
    }

    pub async fn revive_expired_not_found(
        &self,
        ttl_days: i64,
        limit: usize,
    ) -> Result<Vec<String>> {
        self.write(move |db| db.revive_expired_not_found(ttl_days, limit))
            .await
    }

    pub async fn prune_not_found(&self, ttl_days: i64) -> Result<usize> {
        self.write(move |db| db.prune_not_found(ttl_days)).await
    }

    pub async fn get_metadata(&self, key: impl Into<String>) -> Result<Option<String>> {
        let key = key.into();
        self.read(move |db| db.get_metadata(&key)).await
    }

    pub async fn set_metadata(
        &self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<()> {
        let key = key.into();
        let value = value.into();
        self.write(move |db| db.set_metadata(&key, &value)).await
    }

    #[allow(dead_code)]
    pub(crate) async fn reserve_wigle_lookup(
        &self,
        daily_limit: u32,
        failures: Arc<std::sync::atomic::AtomicU64>,
        corruption_denials: Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<QuotaReservation> {
        self.reserve_wigle_lookup_inner(
            daily_limit,
            failures,
            corruption_denials,
            ReservationTestHooks::default(),
        )
        .await
    }

    #[allow(dead_code)]
    async fn reserve_wigle_lookup_inner(
        &self,
        daily_limit: u32,
        failures: Arc<std::sync::atomic::AtomicU64>,
        corruption_denials: Arc<std::sync::atomic::AtomicU64>,
        hooks: ReservationTestHooks,
    ) -> Result<QuotaReservation> {
        let ReservationTestHooks {
            queued,
            committed: committed_signal,
            release_after_commit,
            decision: decision_signal,
            release_after_decision,
        } = hooks;
        let committed = Arc::new(AtomicBool::new(false));
        // Construct ownership before enqueue/await. If this future is dropped,
        // the lease appends its conditional refund behind this command.
        let mut lease = QuotaLease::new(self.writer.clone(), Arc::clone(&committed), failures);
        let operation_committed = Arc::clone(&committed);
        let reservation = Self::call_cancellable(
            &self.writer,
            move |db| {
                let decision = db.try_reserve_wigle_lookup(daily_limit)?;
                if decision == QuotaDecision::CounterCorrupt {
                    // Count the executed denial before publishing its result.
                    // A waiter may disappear after this point, but the actor
                    // still observed and denied one reservation attempt.
                    corruption_denials.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                if decision == QuotaDecision::Reserved {
                    // The DB method returns Reserved only after COMMIT succeeds.
                    operation_committed.store(true, Ordering::Release);
                    if let Some(signal) = committed_signal {
                        let _ = signal.send(());
                    }
                    if let Some(release) = release_after_commit {
                        release.recv().map_err(|_| {
                            anyhow::anyhow!("reservation test release channel closed")
                        })?;
                    }
                }
                if let Some(signal) = decision_signal {
                    let _ = signal.send(decision);
                }
                if let Some(release) = release_after_decision {
                    release
                        .recv()
                        .map_err(|_| anyhow::anyhow!("reservation decision release closed"))?;
                }
                Ok(decision)
            },
            queued,
        )
        .await;

        match reservation {
            Ok(QuotaDecision::Reserved) => Ok(QuotaReservation::Reserved(lease)),
            Ok(QuotaDecision::Exhausted) => {
                lease.disarm();
                Ok(QuotaReservation::Exhausted)
            }
            Ok(QuotaDecision::CounterCorrupt) => {
                lease.disarm();
                Ok(QuotaReservation::CounterCorrupt)
            }
            Err(error) => {
                // A normal DB error occurs before a successful commit and can
                // be defused. If a post-commit test gate failed, keep the lease
                // armed so Drop repairs the committed reservation.
                if !committed.load(Ordering::Acquire) {
                    lease.disarm();
                }
                Err(error)
            }
        }
    }

    #[cfg(test)]
    async fn reserve_wigle_lookup_with_hooks(
        &self,
        daily_limit: u32,
        failures: Arc<std::sync::atomic::AtomicU64>,
        corruption_denials: Arc<std::sync::atomic::AtomicU64>,
        hooks: ReservationTestHooks,
    ) -> Result<QuotaReservation> {
        self.reserve_wigle_lookup_inner(daily_limit, failures, corruption_denials, hooks)
            .await
    }

    /// A FIFO writer round-trip used to observe preceding fire-and-forget
    /// commands (notably quota refunds) without sleeps.
    #[cfg(test)]
    pub async fn writer_barrier(&self) -> Result<()> {
        self.write_durable(|_| Ok(())).await
    }

    #[cfg(test)]
    pub async fn hold_writer_until(
        &self,
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    ) -> Result<()> {
        self.write_durable(move |_| {
            let _ = entered.send(());
            release
                .recv()
                .map_err(|_| anyhow::anyhow!("test writer release channel closed"))?;
            Ok(())
        })
        .await
    }

    #[cfg(test)]
    pub(crate) async fn hold_reader_until(
        &self,
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    ) -> Result<()> {
        Self::call_durable(&self.reader, move |_| {
            let _ = entered.send(());
            release
                .recv()
                .map_err(|_| anyhow::anyhow!("test reader release channel closed"))?;
            Ok(())
        })
        .await
    }

    #[cfg(test)]
    async fn queued_reader_probe(
        &self,
        executed: Arc<AtomicBool>,
        queued: tokio::sync::oneshot::Sender<()>,
    ) -> Result<()> {
        Self::call_interruptible(
            &self.reader,
            move |_| {
                executed.store(true, Ordering::SeqCst);
                Ok(())
            },
            Some(queued),
        )
        .await
    }

    #[cfg(test)]
    async fn actor_observations(
        &self,
    ) -> Result<((String, String, i64, u64), (String, String, i64, u64))> {
        let writer = self
            .write(|db| {
                Ok((
                    std::thread::current()
                        .name()
                        .unwrap_or("unnamed")
                        .to_owned(),
                    db.conn
                        .pragma_query_value(None, "journal_mode", |row| row.get(0))?,
                    db.conn
                        .pragma_query_value(None, "synchronous", |row| row.get(0))?,
                    db.conn
                        .pragma_query_value(None, "busy_timeout", |row| row.get(0))?,
                ))
            })
            .await?;
        let reader = self
            .read(|db| {
                Ok((
                    std::thread::current()
                        .name()
                        .unwrap_or("unnamed")
                        .to_owned(),
                    db.conn
                        .pragma_query_value(None, "journal_mode", |row| row.get(0))?,
                    db.conn
                        .pragma_query_value(None, "synchronous", |row| row.get(0))?,
                    db.conn
                        .pragma_query_value(None, "busy_timeout", |row| row.get(0))?,
                ))
            })
            .await?;
        Ok((writer, reader))
    }

    #[cfg(test)]
    async fn set_metadata_started(
        &self,
        key: String,
        value: String,
        started: tokio::sync::oneshot::Sender<()>,
    ) -> Result<()> {
        self.write_durable(move |db| {
            let _ = started.send(());
            db.set_metadata(&key, &value)
        })
        .await
    }

    #[cfg(test)]
    async fn set_metadata_queued(
        &self,
        key: String,
        value: String,
        queued: tokio::sync::oneshot::Sender<()>,
    ) -> Result<()> {
        Self::call_cancellable(
            &self.writer,
            move |db| db.set_metadata(&key, &value),
            Some(queued),
        )
        .await
    }

    #[cfg(test)]
    async fn run_long_read(&self, started: tokio::sync::oneshot::Sender<()>) -> Result<i64> {
        self.read(move |db| {
            let _ = started.send(());
            db.conn
                .query_row(
                    "WITH RECURSIVE counter(x) AS (
                         VALUES(0) UNION ALL SELECT x + 1 FROM counter WHERE x < 1000000000
                     ) SELECT sum(x) FROM counter",
                    [],
                    |row| row.get(0),
                )
                .map_err(Into::into)
        })
        .await
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) async fn install_writer_commit_counter(
        &self,
        commits: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<()> {
        self.write_durable(move |db| {
            db.conn.commit_hook(Some(move || {
                commits.fetch_add(1, Ordering::SeqCst);
                false
            }));
            Ok(())
        })
        .await
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) async fn set_cached_last_seen_for_test(
        &self,
        bssids: Vec<String>,
        value: String,
    ) -> Result<()> {
        self.write_durable(move |db| {
            let tx = db
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            for bssid in bssids {
                tx.execute(
                    "UPDATE aps SET last_seen = ?1 WHERE bssid = ?2",
                    params![value, bssid],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) async fn cached_last_seen_for_test(
        &self,
        bssids: Vec<String>,
    ) -> Result<Vec<String>> {
        self.read(move |db| {
            bssids
                .into_iter()
                .map(|bssid| {
                    db.conn
                        .query_row(
                            "SELECT last_seen FROM aps WHERE bssid = ?1",
                            params![bssid],
                            |row| row.get(0),
                        )
                        .map_err(Into::into)
                })
                .collect()
        })
        .await
    }

    #[cfg(test)]
    async fn hold_writer_transaction_until(
        &self,
        entered: tokio::sync::oneshot::Sender<()>,
        release: mpsc::Receiver<()>,
    ) -> Result<()> {
        self.write(move |db| {
            let tx = db
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            // Make the transaction a real WAL writer before publishing the
            // deterministic test boundary.
            tx.execute(
                "INSERT INTO metadata (key, value) VALUES ('actor_test_lock', '1')
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [],
            )?;
            let _ = entered.send(());
            release
                .recv()
                .map_err(|_| anyhow::anyhow!("test transaction release channel closed"))?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn stats(&self) -> Result<DbStats> {
        self.read(|db| {
            Ok(DbStats {
                cached_aps: db.cached_ap_count()?,
                pending_aps: db.pending_ap_count()?,
                not_found_aps: db.not_found_ap_count()?,
                db_size_bytes: db.db_size_bytes()?,
                wigle_lookups_today: db.wigle_lookups_today_for_stats()?,
            })
        })
        .await
    }

    #[cfg(test)]
    pub async fn wigle_lookups_today(&self) -> Result<u32> {
        self.read(|db| db.wigle_lookups_today()).await
    }

    pub async fn get_last_fix(&self) -> Result<Option<LastFix>> {
        self.read(|db| db.get_last_fix()).await
    }

    pub async fn set_last_fix(&self, fix: LastFix) -> Result<()> {
        self.write_durable(move |db| db.set_last_fix(&fix)).await
    }

    pub async fn persist_fix_and_history(&self, fix: LastFix, quality: FixQuality) -> Result<()> {
        self.write_durable(move |db| db.persist_fix_and_history(&fix, &quality))
            .await
    }

    /// Persist an autonomous observation and report whether the monotonic
    /// `last_fix` UPSERT accepted its timestamp. The history row commits in
    /// either case; `false` means a newer wall-clock fix remained authoritative.
    pub async fn persist_autolocate_fix_and_history(
        &self,
        fix: LastFix,
        quality: FixQuality,
    ) -> Result<bool> {
        self.write_durable(move |db| {
            let outcome = db.persist_fix_and_history_outcome(&fix, &quality)?;
            let applied = matches!(outcome, LastFixUpdate::Applied);
            warn_if_last_fix_dropped(outcome);
            Ok(applied)
        })
        .await
    }

    /// Delete every persisted privacy-bearing row in one durable transaction.
    /// Once enqueued, dropping the async waiter does not cancel the deletion.
    #[allow(dead_code)]
    pub async fn wipe_privacy_data(&self) -> Result<PrivacyWipeCounts> {
        self.write_durable(Database::wipe_privacy_data).await
    }

    /// Run a synchronous continuation on the writer actor immediately after a
    /// successful wipe commit. The caller's invariant `guard` remains outside
    /// the continuation's inner unwind boundary and is returned on success.
    /// Both remain owned by the durable actor command if the async waiter is
    /// cancelled. On DB error the continuation is dropped without running and
    /// the guard is released normally. A panic is process-fatal while the guard
    /// is still held: after COMMIT, recovering only the actor could expose empty
    /// persisted state alongside stale managed memory.
    #[allow(dead_code)]
    pub(crate) async fn wipe_privacy_data_then<R, G, F>(
        &self,
        guard: G,
        after_commit: F,
        queued: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<(PrivacyWipeCounts, R, G)>
    where
        R: Send + 'static,
        G: Send + 'static,
        F: FnOnce(&mut G) -> R + Send + 'static,
    {
        let ((counts, continuation), guard) = Self::call_durable_fail_stop_guarded_queued(
            &self.writer,
            "privacy wipe committed-state continuation",
            guard,
            move |db, guard| {
                let counts = db.wipe_privacy_data()?;
                Ok((counts, after_commit(guard)))
            },
            queued,
        )
        .await?;
        Ok((counts, continuation, guard))
    }

    #[cfg(test)]
    async fn wipe_privacy_data_queued(
        &self,
        queued: tokio::sync::oneshot::Sender<()>,
    ) -> Result<PrivacyWipeCounts> {
        Self::call_durable_queued(&self.writer, Database::wipe_privacy_data, Some(queued)).await
    }

    // `db.rs` is compiled into both the library and daemon targets. The
    // daemon uses the guarded variant; DB unit tests use this plain variant.
    #[allow(dead_code)]
    pub async fn get_fixes_in_range(
        &self,
        start: impl Into<String>,
        end: impl Into<String>,
    ) -> Result<Vec<Fix>> {
        let start = start.into();
        let end = end.into();
        self.read(move |db| db.get_fixes_in_range(&start, &end))
            .await
    }

    /// History admission variant: `guard` is captured by the reader command
    /// itself, so cancelling the async waiter cannot release single-flight
    /// admission while SQLite is still unwinding an interrupted query.
    // The library target intentionally does not include server.rs, while the
    // daemon target calls this method from its history handler.
    #[allow(dead_code)]
    pub(crate) async fn get_fixes_in_range_guarded<G>(
        &self,
        start: impl Into<String>,
        end: impl Into<String>,
        guard: G,
    ) -> Result<Vec<Fix>>
    where
        G: Send + 'static,
    {
        self.get_fixes_in_range_guarded_inner(start.into(), end.into(), guard, None, || {})
            .await
    }

    /// Bounded raw-history variant. The admission guard is owned by the
    /// interruptible reader command, exactly like segmented history, so an
    /// aborted async waiter cannot release the single-flight slot before the
    /// SQLite transaction has unwound.
    #[allow(dead_code)]
    pub(crate) async fn get_capped_fixes_in_range_guarded<G>(
        &self,
        start: impl Into<String>,
        end: impl Into<String>,
        max_fixes: u64,
        guard: G,
    ) -> Result<CappedFixRows>
    where
        G: Send + 'static,
    {
        let start = start.into();
        let end = end.into();
        Self::call_interruptible(
            &self.reader,
            move |db| {
                let guard = guard;
                let result = db.get_capped_fixes_in_range(&start, &end, max_fixes);
                drop(guard);
                result
            },
            None,
        )
        .await
    }

    /// Exact guarded reader core. Test hooks are per-call values captured by
    /// the same interruptible actor command as production: `queued` fires only
    /// after channel insertion, and `before_query` runs on the reader thread
    /// with the admission guard already live. No process-global test state.
    async fn get_fixes_in_range_guarded_inner<G, H>(
        &self,
        start: String,
        end: String,
        guard: G,
        queued: Option<tokio::sync::oneshot::Sender<()>>,
        before_query: H,
    ) -> Result<Vec<Fix>>
    where
        G: Send + 'static,
        H: FnOnce() + Send + 'static,
    {
        Self::call_interruptible(
            &self.reader,
            move |db| {
                let guard = guard;
                before_query();
                let result = db.get_fixes_in_range(&start, &end);
                // Explicitly retain admission through query completion/error;
                // an aborted async waiter must not admit overlapping DB work.
                drop(guard);
                result
            },
            queued,
        )
        .await
    }

    #[cfg(test)]
    async fn get_fixes_in_range_guarded_with_hooks<G, H>(
        &self,
        start: impl Into<String>,
        end: impl Into<String>,
        guard: G,
        queued: Option<tokio::sync::oneshot::Sender<()>>,
        before_query: H,
    ) -> Result<Vec<Fix>>
    where
        G: Send + 'static,
        H: FnOnce() + Send + 'static,
    {
        self.get_fixes_in_range_guarded_inner(start.into(), end.into(), guard, queued, before_query)
            .await
    }

    #[allow(dead_code)]
    pub async fn prune_fixes(&self, retention_days: i64) -> Result<usize> {
        let Some(cutoff) = retention_cutoff(retention_days)? else {
            return Ok(0);
        };
        let mut total = 0;
        loop {
            let cutoff = cutoff.clone();
            let deleted = self
                .write(move |db| db.prune_fixes_chunk(&cutoff, RETENTION_CHUNK_SIZE))
                .await?;
            total += deleted;
            if deleted < RETENTION_CHUNK_SIZE {
                return Ok(total);
            }
            // Re-enqueue the next chunk at the tail so quota reservations and
            // shutdown metadata already waiting on the writer get a turn.
            tokio::task::yield_now().await;
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn prune_fixes_before_chunk(
        &self,
        cutoff: impl Into<String>,
    ) -> Result<usize> {
        let cutoff = cutoff.into();
        self.write(move |db| db.prune_fixes_chunk(&cutoff, RETENTION_CHUNK_SIZE))
            .await
    }

    #[allow(dead_code)]
    pub async fn prune_aps(&self, retention_days: i64) -> Result<usize> {
        let Some(cutoff) = retention_cutoff(retention_days)? else {
            return Ok(0);
        };
        let mut total = 0;
        loop {
            let cutoff = cutoff.clone();
            let deleted = self
                .write(move |db| db.prune_aps_chunk(&cutoff, RETENTION_CHUNK_SIZE))
                .await?;
            total += deleted;
            if deleted < RETENTION_CHUNK_SIZE {
                return Ok(total);
            }
            tokio::task::yield_now().await;
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn prune_aps_before_chunk(&self, cutoff: impl Into<String>) -> Result<usize> {
        let cutoff = cutoff.into();
        self.write(move |db| db.prune_aps_chunk(&cutoff, RETENTION_CHUNK_SIZE))
            .await
    }

    pub async fn debug_statuses(
        &self,
        bssids: Vec<String>,
        ttl_days: i64,
    ) -> Result<Vec<(String, DebugDbStatus)>> {
        self.read(move |db| {
            bssids
                .into_iter()
                .map(|bssid| {
                    let status = if db.get_ap(&bssid)?.is_some() {
                        DebugDbStatus::Cached
                    } else if db.is_not_found(&bssid, ttl_days)? {
                        DebugDbStatus::NotFound
                    } else if db.is_pending(&bssid)? {
                        DebugDbStatus::Pending
                    } else {
                        DebugDbStatus::New
                    };
                    Ok((bssid, status))
                })
                .collect()
        })
        .await
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DebugDbStatus {
    Cached,
    NotFound,
    Pending,
    New,
}

const SCHEMA_VERSION: i32 = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CorruptWigleLookupsValue {
    raw: String,
}

impl std::fmt::Display for CorruptWigleLookupsValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "metadata.wigle_lookups_today is not a valid u32 ({:?})",
            self.raw
        )
    }
}

impl std::error::Error for CorruptWigleLookupsValue {}

/// Parse the `metadata.wigle_lookups_today` raw value without logging or applying
/// caller policy. `None` is the legitimate zero-lookup state; a malformed value
/// remains available to the caller for an escaped, context-specific warning.
fn parse_wigle_lookups_value(
    raw: Option<String>,
) -> std::result::Result<u32, CorruptWigleLookupsValue> {
    match raw {
        None => Ok(0),
        Some(raw) => raw
            .parse::<u32>()
            .map_err(|_| CorruptWigleLookupsValue { raw }),
    }
}

/// Compute one fixed UTC cutoff for a retention sweep. `0` disables pruning;
/// negative or over-policy values fail safe instead of turning into a future
/// cutoff or overflowing Chrono arithmetic.
pub fn retention_cutoff(retention_days: i64) -> Result<Option<String>> {
    if retention_days == 0 {
        return Ok(None);
    }
    if !(1..=MAX_RETENTION_DAYS).contains(&retention_days) {
        anyhow::bail!("retention days must be in 0..={MAX_RETENTION_DAYS}, got {retention_days}");
    }
    let delta = chrono::TimeDelta::try_days(retention_days)
        .ok_or_else(|| anyhow::anyhow!("retention duration is not representable"))?;
    let cutoff = Utc::now()
        .checked_sub_signed(delta)
        .ok_or_else(|| anyhow::anyhow!("retention cutoff is not representable"))?;
    Ok(Some(cutoff.to_rfc3339()))
}

/// A historical fix from the `fixes` table (task-0031). Each records one
/// successful locate response so the daemon can answer "where was I N days
/// ago" queries by grouping fixes into stay-point segments.
///
/// TERMINOLOGY (task-0131): "fix" here is a **Wi-Fi-trilateration-derived
/// position estimate, NOT a GPS fix**. This daemon never sees GNSS/satellite
/// data — every position is inferred from the geolocation of visible Wi-Fi
/// APs. In particular `accuracy_m` is a trilateration confidence radius, not a
/// GPS HDOP-derived value (task-0127); a reader from a GPS background must not
/// assume satellite semantics.
///
/// task-0134: this is a DOMAIN type — the DB layer owns the on-disk RFC3339
/// string and hands callers an already-parsed `at: DateTime<Utc>`. History
/// segmentation therefore never re-parses a timestamp per iteration, and a
/// corrupt persisted timestamp becomes a typed read error at this boundary
/// rather than a silently-defaulted zero duration.
///
/// The DB `id` is not a field here: it is only needed transiently as the
/// row-id in corrupt-row error messages (see `parse_fix_timestamp` /
/// `decode_fix_quality`), never by any consumer of the typed struct, so it is
/// not carried (task-0132 dropped it). `n_sources` IS consumed — the
/// raw-history `HistoryFix` conversion (`server.rs`) surfaces it, and
/// stay-point segmentation (`history::flush_segment`, task-0132) averages it
/// into `HistorySegment::mean_n_sources` so callers can filter low-confidence
/// stays.
#[derive(Debug, Clone)]
pub struct Fix {
    pub at: DateTime<Utc>,
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: f64,
    /// Number of cached APs used to compute this fix.
    pub n_sources: i64,
    /// `None` is reserved for a legacy row whose five quality columns are all
    /// NULL. Any partial or malformed persisted bundle is a read error.
    ///
    /// task-0132: narrow field-level allow (replacing the former struct-level
    /// blanket). This is the only `pub(crate)` field, so it is the only one the
    /// lib-target dead-code pass can flag: its sole production reader is the
    /// binary crate's `From<db::Fix> for HistoryFix` (server.rs), which is not
    /// linked into the lib target (`lib.rs` re-exports `db` for tests but not
    /// `server`). The `pub` fields above are public API and never flagged.
    #[allow(dead_code)]
    pub(crate) quality: Option<FixQuality>,
}

/// Parse a `fixes.at_rfc3339` string to `DateTime<Utc>` inside a rusqlite row
/// closure (task-0134). A malformed timestamp is a corrupt-row read error,
/// carrying the row id, mirroring [`corrupt_fix_quality`]. This replaces the
/// former silent `_ => 0` default that `history::segment_fixes` applied.
fn parse_fix_timestamp(row_id: i64, at_rfc3339: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at_rfc3339)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|error| {
            corrupt_fix_quality(
                row_id,
                format!("unparseable timestamp {at_rfc3339:?}: {error}"),
            )
        })
}

fn corrupt_fix_quality(row_id: i64, message: impl std::fmt::Display) -> rusqlite::Error {
    // The column index (10 = suspect_reason) and Type::Text are synthetic
    // placeholders required by the rusqlite error shape; the malformed field may
    // actually be a REAL. The authoritative context is the row-id + reason in
    // the boxed message, which callers surface and tests assert on — do not
    // read the ordinal/type as the true offending column.
    rusqlite::Error::FromSqlConversionFailure(
        10,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("corrupt fixes row id {row_id}: {message}"),
        )),
    )
}

fn decode_fix_quality(
    row_id: i64,
    implied_speed_kmh: Option<f64>,
    last_fix_age_secs: Option<i64>,
    inlier_ratio: Option<f64>,
    confidence: Option<String>,
    suspect_reason: Option<String>,
) -> rusqlite::Result<Option<FixQuality>> {
    let all_null = implied_speed_kmh.is_none()
        && last_fix_age_secs.is_none()
        && inlier_ratio.is_none()
        && confidence.is_none()
        && suspect_reason.is_none();
    if all_null {
        return Ok(None);
    }
    let confidence = confidence.ok_or_else(|| {
        corrupt_fix_quality(
            row_id,
            "quality fields are present without the required confidence",
        )
    })?;
    let confidence =
        Confidence::from_db_str(&confidence).map_err(|error| corrupt_fix_quality(row_id, error))?;
    FixQuality::new(
        implied_speed_kmh,
        last_fix_age_secs,
        inlier_ratio,
        confidence,
        suspect_reason,
    )
    .map(Some)
    .map_err(|error| corrupt_fix_quality(row_id, error))
}

/// A bounded newest-page of raw history fixes. `fixes` is returned in
/// chronological order; `elided` is present only when earlier matching rows
/// were omitted by the cap.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct CappedFixRows {
    pub fixes: Vec<Fix>,
    pub elided: Option<i64>,
}

/// A persisted last-known position (task-0134 domain type; formerly split into
/// an on-disk `LastFixRow` + an in-memory `server::LastFix`). The DB layer owns
/// the RFC3339<->`DateTime<Utc>` marshalling so callers never hand-marshal, and
/// wall-clock age across restarts is computed directly on `at`.
///
/// TERMINOLOGY (task-0131): like [`Fix`], this "fix" is a
/// **Wi-Fi-trilateration-derived position estimate, NOT a GPS fix** — the
/// daemon has no GNSS input. `accuracy_m` is a trilateration confidence radius,
/// not a GPS HDOP-derived value (task-0127).
///
/// task-0238 non-finite guard: a persisted row with a non-finite coordinate or
/// accuracy, or an unparseable timestamp, is rejected at read time by
/// [`last_fix_from_columns`] and never materialises into a `LastFix`.
#[derive(Debug, Clone)]
pub struct LastFix {
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: f64,
    pub address: Option<String>,
    pub at: DateTime<Utc>,
    pub sources: usize,
}

/// A persisted `last_fix` row was read but is unusable — a non-finite
/// coordinate/accuracy (task-0238) or an unparseable timestamp — and was
/// discarded rather than served. This is a DISTINCT error category from a
/// SQLite read failure so callers can branch:
///
/// - startup rehydration (`main.rs`) discards either kind as `None` + a warn;
/// - the locate path (`server.rs`) ignores a corrupt row and still answers,
///   but aborts on a genuine read failure.
///
/// `get_last_fix` returns this (wrapped in `anyhow::Error`) so callers use
/// `error.downcast_ref::<CorruptLastFix>()` to tell the two apart.
#[derive(Debug)]
pub struct CorruptLastFix(String);

impl std::fmt::Display for CorruptLastFix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CorruptLastFix {}

/// Convert the six raw `last_fix` columns into the domain [`LastFix`],
/// applying the task-0238 non-finite guard and parsing the RFC3339 timestamp.
/// This is the single Row->domain choke point (relocated from the former
/// `TryFrom<LastFixRow> for server::LastFix`); a rejected row surfaces as a
/// [`CorruptLastFix`] so it is discarded, never served.
fn last_fix_from_columns(
    lat: f64,
    lon: f64,
    accuracy_m: f64,
    address: Option<String>,
    at_rfc3339: String,
    sources: i64,
) -> Result<LastFix> {
    // INVARIANT: every rejection here MUST surface `CorruptLastFix` as the
    // TOP-LEVEL anyhow error. Do NOT `.context(..)`-wrap it (here, in
    // `get_last_fix`, or in the DbHandle `read` plumbing): `handle_locate` and
    // `main.rs` branch on `error.downcast_ref::<CorruptLastFix>()` to
    // distinguish a poisoned-but-ignorable row from a genuine read failure.
    // A context wrapper would reparent the chain, defeat the downcast, and
    // silently turn a corrupt last_fix into a hard locate abort.
    //
    // task-0238: SQLite REAL columns can contain NaN/±Inf. Reject the persisted
    // row here so it is discarded (and the cause warned) instead of exposing a
    // poisoned stale fix to return_last_fix_or_error.
    if !lat.is_finite() || !lon.is_finite() {
        return Err(CorruptLastFix(format!(
            "non-finite last_fix coordinates: lat={lat}, lon={lon}"
        ))
        .into());
    }
    if !accuracy_m.is_finite() || accuracy_m < 0.0 {
        return Err(CorruptLastFix(format!("invalid last_fix accuracy_m: {accuracy_m}")).into());
    }
    let at = DateTime::parse_from_rfc3339(&at_rfc3339)
        .map_err(|error| {
            CorruptLastFix(format!(
                "unparseable last_fix timestamp {at_rfc3339:?}: {error}"
            ))
        })?
        .with_timezone(&Utc);
    Ok(LastFix {
        lat,
        lon,
        accuracy_m,
        address,
        at,
        sources: sources.max(0) as usize,
    })
}

const UPSERT_LAST_FIX_SQL: &str =
    "INSERT INTO last_fix (id, lat, lon, accuracy_m, address, at_rfc3339, sources)
     VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
     ON CONFLICT(id) DO UPDATE SET
        lat        = excluded.lat,
        lon        = excluded.lon,
        accuracy_m = excluded.accuracy_m,
        address    = excluded.address,
        at_rfc3339 = excluded.at_rfc3339,
        sources    = excluded.sources
     WHERE excluded.at_rfc3339 >= last_fix.at_rfc3339";

#[derive(Debug, Clone, PartialEq, Eq)]
enum LastFixUpdate {
    Applied,
    DroppedOlder { incoming: String, stored: String },
}

fn upsert_last_fix(connection: &Connection, fix: &LastFix) -> Result<LastFixUpdate> {
    // task-0134/task-0238 invariant: the byte-identical `DateTime::<Utc>::
    // to_rfc3339()` format is what the monotonic UPSERT's lexical `>=` compare
    // (and the range-query string bounds) rely on. Marshal it HERE, inside the
    // DB layer, so callers pass a domain `at: DateTime<Utc>` and cannot vary the
    // on-disk format.
    let at_rfc3339 = fix.at.to_rfc3339();
    let changed = connection.execute(
        UPSERT_LAST_FIX_SQL,
        params![
            fix.lat,
            fix.lon,
            fix.accuracy_m,
            fix.address,
            at_rfc3339,
            fix.sources as i64
        ],
    )?;
    match changed {
        1 => Ok(LastFixUpdate::Applied),
        0 => {
            let stored = connection.query_row(
                "SELECT at_rfc3339 FROM last_fix WHERE id = 1",
                [],
                |row| row.get(0),
            )?;
            Ok(LastFixUpdate::DroppedOlder {
                incoming: at_rfc3339,
                stored,
            })
        }
        changed => anyhow::bail!("last_fix UPSERT changed an unexpected {changed} rows"),
    }
}

fn warn_if_last_fix_dropped(outcome: LastFixUpdate) {
    if let LastFixUpdate::DroppedOlder { incoming, stored } = outcome {
        tracing::warn!(
            "persisted last_fix update rejected as older: incoming timestamp {:?}, \
             stored timestamp {:?}; memory/history may still contain the incoming observation",
            incoming,
            stored
        );
    }
}

/// task-0197: tighten the SQLite database file (and any pre-existing -wal/-shm
/// sidecars) to mode 0600. Best-effort: a chmod failure is warned, never
/// fatal, so the daemon can still serve reads on a read-only filesystem.
/// Called on every open BEFORE WAL is enabled, so sidecars SQLite creates
/// afterward inherit the tightened mode (SQLite copies the DB file's perms).
/// Unix-only by construction — whereamid is Linux-only (tokio unix signals,
/// nmcli/iw).
fn tighten_db_file_permissions(path: &Path) {
    tighten_file_to_0600(path);
    // Sidecars from a prior run may still be group/other-readable; tighten
    // them if present (SQLite names them "<db>-wal" / "<db>-shm").
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        tighten_file_to_0600(Path::new(&sidecar));
    }
}

/// chmod a single file to 0600. A missing file is a silent no-op (an absent
/// sidecar is normal); any other error warns and is swallowed. After a
/// successful chmod the mode is re-checked: some filesystems (FUSE, network
/// mounts) accept chmod and ignore it, and a location-history DB that stays
/// world-readable must never be SILENTLY loose (task-0197 review).
fn tighten_file_to_0600(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        Ok(()) => {
            if let Ok(meta) = std::fs::metadata(path) {
                let mode = meta.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    tracing::warn!(
                        "{} remains group/other-accessible (mode {mode:o}) after \
                         chmod 0600 — the filesystem ignored the permission change; \
                         it may be readable by other local users",
                        path.display()
                    );
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            tracing::warn!(
                "could not tighten permissions on {} to 0600 ({e}); \
                 it may be readable by other local users",
                path.display()
            );
        }
    }
}

impl Database {
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn execute_batch_for_test(&self, sql: &str) -> Result<()> {
        self.conn.execute_batch(sql)?;
        Ok(())
    }

    /// Open or create the database at `path`. Sets WAL mode and creates schema if needed.
    #[allow(dead_code)]
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_mobile_ssid_denylist(path, crate::scanner::default_mobile_ssid_denylist())
    }

    fn open_with_mobile_ssid_denylist(
        path: &Path,
        mobile_ssid_denylist: Vec<String>,
    ) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database at {}", path.display()))?;

        // task-0197: tighten the DB file to owner-only (0600) on EVERY open,
        // BEFORE the journal_mode pragma below activates WAL. SQLite copies the
        // main database file's permission bits onto the -wal/-shm sidecars it
        // creates, so tightening first means those sidecars are born 0600 too.
        // On an existing DB left loose by an older daemon this is also the
        // AC#3 "existing databases are tightened on startup" path. Best-effort:
        // a chmod failure warns and continues so a read-only filesystem can
        // still serve read queries.
        //
        // NOTE (task-0192): this chmod MUST stay before the journal_mode
        // pragma — reordering it after WAL activation would let the sidecars be
        // created group/other-readable before we tighten the main file.
        tighten_db_file_permissions(path);

        let wal_mode: String = conn
            .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
            .context("failed to set WAL mode")?;
        if wal_mode.to_lowercase() != "wal" {
            tracing::warn!(
                "WAL mode requested but got '{}' — performance may be degraded",
                wal_mode
            );
        }
        conn.pragma_update(None, "synchronous", "NORMAL")
            .context("failed to set synchronous=NORMAL")?;
        conn.busy_timeout(DB_BUSY_TIMEOUT)
            .context("failed to set SQLite busy timeout")?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .context("failed to enable foreign keys")?;

        let db = Self {
            conn,
            mobile_ssid_denylist,
        };
        db.migrate()?;
        Ok(db)
    }

    /// Open the independent read actor only after the writer has completed
    /// migrations. It is intentionally a normal WAL connection (rather than
    /// SQLite read-only mode) because connection-local pragmas such as
    /// `synchronous` cannot be reliably configured on every SQLite build for
    /// a read-only handle. The actor API exposes read operations only.
    fn open_reader(path: &Path) -> Result<Self> {
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        // Deliberately omit SQLITE_OPEN_CREATE. The writer must have opened
        // and migrated this exact file first; a typo/race must fail instead of
        // silently creating an empty reader database.
        let conn = Connection::open_with_flags(path, flags)
            .with_context(|| format!("failed to open database reader at {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .context("failed to enable WAL on database reader")?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .context("failed to set reader synchronous=NORMAL")?;
        conn.busy_timeout(DB_BUSY_TIMEOUT)
            .context("failed to set reader SQLite busy timeout")?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .context("failed to enable reader foreign keys")?;
        Ok(Self {
            conn,
            mobile_ssid_denylist: Vec::new(),
        })
    }

    #[cfg(test)]
    fn open_shared_memory(uri: &str, migrate: bool) -> Result<Self> {
        use rusqlite::OpenFlags;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI;
        let conn = Connection::open_with_flags(uri, flags)?;
        conn.busy_timeout(DB_BUSY_TIMEOUT)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Self {
            conn,
            mobile_ssid_denylist: crate::scanner::default_mobile_ssid_denylist(),
        };
        if migrate {
            db.migrate()?;
        }
        Ok(db)
    }

    /// Open an in-memory database (for testing).
    #[allow(dead_code)]
    pub fn open_memory() -> Result<Self> {
        Self::open_memory_with_mobile_ssid_denylist(crate::scanner::default_mobile_ssid_denylist())
    }

    fn open_memory_with_mobile_ssid_denylist(mobile_ssid_denylist: Vec<String>) -> Result<Self> {
        let conn = Connection::open_in_memory().context("failed to open in-memory database")?;
        let db = Self {
            conn,
            mobile_ssid_denylist,
        };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let mut version = self.get_schema_version();

        // task-0191 AC#3: heal a DB whose stored version is newer than its
        // actual table shape. This is exactly the corruption the old
        // migrate_v2_to_v3 SCHEMA_VERSION-stamping bug could produce: a crash
        // between the v2->v3 commit and the v3->v4 commit left a final-version
        // stamp on disk with neither last_fix (v4) nor fixes (v5) present,
        // after which every later open accepted the stamp, skipped the missing
        // migrations, and stranded the daemon permanently with no self-heal.
        //
        // Repair = reset the stored version DOWN to the highest version whose
        // shape is actually satisfied, then let the normal chain below re-run
        // and rebuild the missing tables. This is safe ONLY because every
        // migration is idempotency-guarded (v1->v2 has_priority probe, v2->v3
        // already_constrained probe, v3->v4/v4->v5 CREATE TABLE IF NOT EXISTS,
        // v5->v6 column probe),
        // so re-running heals rather than harms.
        //
        // Guard rail: only repair when version <= SCHEMA_VERSION. A version
        // STRICTLY greater than SCHEMA_VERSION is a genuinely-newer schema
        // written by a future binary; its tables are simply unknown to us and
        // must NOT be "repaired" down (that would corrupt a forward-compatible
        // DB). The forward-compat warning at the end of migrate() handles that
        // case instead. A missing shape can only be observed at v4+
        // (last_fix/fixes/is_mobile); v1..v3 touch only tables
        // create_schema_v1 always creates, so there is no sub-v4 shape to lose.
        if version <= SCHEMA_VERSION {
            let satisfied = self.highest_satisfied_shape_version(version);
            if satisfied < version {
                tracing::warn!(
                    "database stamped v{version} but its table shape only satisfies \
                     v{satisfied} (missing a required table/column); resetting the stored \
                     version down and re-running migrations to heal (task-0191)"
                );
                // A stored version >= 4 implies the schema_version table
                // already has the v3 single-row (id = 1) shape — reaching v4
                // required migrate_v3_to_v4, which addresses the row by id.
                // (This block only fires when satisfied < version, i.e.
                // version >= 4.) A schema_version table manually corrupted
                // out of that contract makes the UPDATE below either error
                // (id column gone) or match zero rows (row re-keyed/absent);
                // in the zero-row case the stale stamp persists for the next
                // open to retry, but the chain below still rebuilds the
                // missing tables either way — it never mis-heals a healthy
                // row.
                self.conn
                    .execute(
                        "UPDATE schema_version SET version = ?1 WHERE id = 1",
                        params![satisfied],
                    )
                    .context("failed to reset schema_version during shape repair")?;
                version = satisfied;
            }
        }

        if version == 0 {
            info!("initializing database schema v{SCHEMA_VERSION}");
            self.create_schema_v1()?;
            version = 1;
        }

        if version < 2 {
            info!("migrating database v{version} -> v2 (add aps.source_priority)");
            self.migrate_v1_to_v2()?;
            version = 2;
        }

        if version < 3 {
            info!("migrating database v{version} -> v3 (single-row schema_version)");
            self.migrate_v2_to_v3()?;
            version = 3;
        }

        if version < 4 {
            info!("migrating database v{version} -> v4 (last_fix table)");
            self.migrate_v3_to_v4()?;
            version = 4;
        }

        if version < 5 {
            info!("migrating database v{version} -> v5 (fixes timeseries table)");
            self.migrate_v4_to_v5()?;
            version = 5;
        }

        if version < 6 {
            info!("migrating database v{version} -> v6 (mobile AP classification)");
            self.migrate_v5_to_v6()?;
            version = 6;
        }

        if version < 7 {
            info!("migrating database v{version} -> v7 (fix quality metadata)");
            self.migrate_v6_to_v7()?;
            version = 7;
        }

        if version < 8 {
            info!("migrating database v{version} -> v8 (rename api_calls_today -> wigle_lookups_today)");
            self.migrate_v7_to_v8()?;
            version = 8;
        }

        if version < 9 {
            info!("migrating database v{version} -> v9 (drop redundant aps/pending.channel; frequency canonical)");
            self.migrate_v8_to_v9()?;
            version = 9;
        }

        if version == SCHEMA_VERSION {
            debug!("database schema is up to date (v{version})");
        } else if version > SCHEMA_VERSION {
            tracing::warn!(
                "database schema is v{version} but binary expects v{SCHEMA_VERSION}; \
                 forward compatibility is not guaranteed"
            );
        }

        Ok(())
    }

    /// Run a migration body inside an `unchecked_transaction`. The body MUST
    /// also stamp schema_version (because the stamp shape differs across
    /// versions: pre-v3 is a multi-row table, v3+ has CHECK(id=1)). On any
    /// error the transaction is rolled back and the DB stays at the prior
    /// version. Task-0047.
    fn run_migration<F>(&self, body: F) -> Result<()>
    where
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<()>,
    {
        let tx = self
            .conn
            .unchecked_transaction()
            .context("failed to start migration transaction")?;
        body(&tx)?;
        tx.commit().context("failed to commit migration")?;
        Ok(())
    }

    /// v1 -> v2: add `aps.source_priority` and backfill from the existing
    /// `source` column. Idempotent: safe to run multiple times because the
    /// schema_version row is the gating condition; we never reach this from
    /// a v2 database. Wrapped in a transaction (task-0047) so a partial
    /// migration cannot leave the DB with the column added but no version
    /// bump (or vice versa).
    fn migrate_v1_to_v2(&self) -> Result<()> {
        self.run_migration(|tx| {
            // Defensive: if the column already exists (e.g. partial earlier run),
            // skip the ALTER. We detect that via PRAGMA table_info.
            let has_priority: bool = {
                let mut stmt = tx.prepare("PRAGMA table_info(aps)")?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
                let mut found = false;
                for row in rows {
                    if row? == "source_priority" {
                        found = true;
                        break;
                    }
                }
                found
            };
            if !has_priority {
                tx.execute(
                    "ALTER TABLE aps ADD COLUMN source_priority INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .context("failed to add aps.source_priority column")?;
            }

            // Backfill priorities from existing source strings. Hardcode the
            // numbers here rather than calling Source::priority(): future code
            // changes to the enum must not silently rewrite historical data.
            // (If you change the priority ladder, write a new migration.)
            //
            // INVARIANT: these constants must EQUAL Source::priority() AT THE
            // TIME THIS MIGRATION SHIPPED. A later renumbering of the enum
            // requires a NEW migration, not editing this one — this frozen SQL
            // must never move. task-0111's consistency test
            // (migrate_v1_to_v2_priorities_match_source_enum) pins this
            // equality: if the enum is renumbered without a new migration, that
            // test fails, telling you to write the migration instead of editing
            // these numbers.
            tx.execute_batch(
                "UPDATE aps SET source_priority = 40 WHERE source = 'apple';
                 UPDATE aps SET source_priority = 30 WHERE source = 'wigle';
                 UPDATE aps SET source_priority = 20 WHERE source = 'beacondb';
                 UPDATE aps SET source_priority = 10 WHERE source = 'manual';
                 UPDATE aps SET source_priority = 0  WHERE source NOT IN ('apple','wigle','beacondb','manual');",
            )?;

            // Bump schema_version. Multi-row shape (no id PK yet). Update if
            // a row exists, otherwise insert.
            let updated = tx.execute("UPDATE schema_version SET version = 2", [])?;
            if updated == 0 {
                tx.execute("INSERT INTO schema_version (version) VALUES (2)", [])?;
            }
            Ok(())
        })
    }

    /// v2 -> v3: rebuild `schema_version` so it can hold at most one row,
    /// keyed by `id = 1`. SQLite cannot ALTER TABLE to add a CHECK
    /// constraint, so we have to recreate the table.
    ///
    /// Behaviour for malformed v2 databases (multiple rows): coalesce to
    /// MAX(version) — pick the highest version any row claims and discard
    /// the others. This is the conservative choice; a smaller value could
    /// trick `migrate()` into re-running an already-applied migration.
    ///
    /// Idempotent guard: detect if the new shape is already present
    /// (someone might run migrate twice through different code paths) and
    /// skip the rebuild in that case.
    fn migrate_v2_to_v3(&self) -> Result<()> {
        // Detect if the table is already single-row constrained. We look at
        // the SQL stored in sqlite_master for the schema_version table.
        let sql_present: Option<String> = self
            .conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let already_constrained = sql_present
            .as_deref()
            .map(|s| s.contains("CHECK (id = 1)"))
            .unwrap_or(false);
        if already_constrained {
            // Idempotent, no-downgrade stamp of the literal 3 — this
            // migration's postcondition version, NOT the moving SCHEMA_VERSION
            // constant (task-0191, mirroring migrate_v1_to_v2's literal 2):
            // stamping SCHEMA_VERSION here strands a crash-interrupted DB at a
            // version whose later-version tables/columns do not exist yet.
            // INSERT OR IGNORE creates the row iff absent (the id = 1 PK makes
            // a second insert a no-op); the guarded UPDATE then advances it to
            // 3 only when it is behind. Together they never downgrade a row a
            // later migration already advanced (if this path is ever reached
            // out of order), and never error on a pre-existing row.
            self.conn.execute(
                "INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, 3)",
                [],
            )?;
            self.conn.execute(
                "UPDATE schema_version SET version = 3 WHERE id = 1 AND version < 3",
                [],
            )?;
            return Ok(());
        }

        // Rebuild atomically. If anything fails the original table survives.
        let tx = self
            .conn
            .unchecked_transaction()
            .context("failed to start transaction for schema_version rebuild")?;
        tx.execute_batch(
            "CREATE TABLE schema_version_new (
                 id      INTEGER PRIMARY KEY CHECK (id = 1),
                 version INTEGER NOT NULL
             );
             INSERT INTO schema_version_new (id, version)
                 VALUES (1, COALESCE((SELECT MAX(version) FROM schema_version), 0));
             DROP TABLE schema_version;
             ALTER TABLE schema_version_new RENAME TO schema_version;",
        )
        .context("failed to rebuild schema_version with single-row constraint")?;

        // Stamp the (now single) row to the literal 3 — this migration's
        // postcondition version, NOT SCHEMA_VERSION (task-0191). The row was
        // just seeded from COALESCE(MAX(old version), 0), which could already
        // be >= 3 if a malformed source held a higher value; the `version < 3`
        // guard keeps that higher value rather than downgrading it. Stamping
        // SCHEMA_VERSION here was the original defect: a crash before the
        // v3->v4 commit left a final-version stamp on disk with no later tables.
        tx.execute(
            "UPDATE schema_version SET version = 3 WHERE id = 1 AND version < 3",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// v3 -> v4: add the `last_fix` table with a single-row CHECK so the
    /// daemon can persist its last-known position across restarts. Idempotent
    /// via `CREATE TABLE IF NOT EXISTS` and a fresh schema_version stamp.
    /// Wrapped in a transaction (task-0047).
    fn migrate_v3_to_v4(&self) -> Result<()> {
        self.run_migration(|tx| {
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS last_fix (
                     id          INTEGER PRIMARY KEY CHECK (id = 1),
                     lat         REAL NOT NULL,
                     lon         REAL NOT NULL,
                     accuracy_m  REAL NOT NULL,
                     address     TEXT,
                     at_rfc3339  TEXT NOT NULL,
                     sources     INTEGER NOT NULL
                 );",
            )
            .context("failed to create last_fix table")?;
            tx.execute(
                "UPDATE schema_version SET version = ?1 WHERE id = 1",
                params![4],
            )?;
            Ok(())
        })
    }

    /// v4 -> v5: add the `fixes` table (timeseries of successful locates)
    /// for the location-history feature (task-0031). Idempotent via
    /// CREATE TABLE / INDEX IF NOT EXISTS. Wrapped in a transaction
    /// (task-0047).
    fn migrate_v4_to_v5(&self) -> Result<()> {
        self.run_migration(|tx| {
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS fixes (
                     id          INTEGER PRIMARY KEY AUTOINCREMENT,
                     at_rfc3339  TEXT NOT NULL,
                     lat         REAL NOT NULL,
                     lon         REAL NOT NULL,
                     accuracy_m  REAL NOT NULL,
                     n_sources   INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_fixes_at ON fixes(at_rfc3339);",
            )
            .context("failed to create fixes table")?;
            tx.execute(
                "UPDATE schema_version SET version = ?1 WHERE id = 1",
                params![5],
            )?;
            Ok(())
        })
    }

    /// v5 -> v6: persist the coarse mobile-SSID classification alongside
    /// each resolved AP. Existing rows intentionally receive the default 0;
    /// they are not retroactively re-evaluated and will be recomputed only on
    /// their next upsert. Wrapped in `run_migration` so the column and version
    /// stamp commit or roll back together.
    fn migrate_v5_to_v6(&self) -> Result<()> {
        self.run_migration(|tx| {
            let has_is_mobile = {
                let mut stmt = tx.prepare("PRAGMA table_info(aps)")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
                let mut found = false;
                for column in rows {
                    if column? == "is_mobile" {
                        found = true;
                        break;
                    }
                }
                found
            };
            if !has_is_mobile {
                tx.execute(
                    "ALTER TABLE aps ADD COLUMN is_mobile INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .context("failed to add aps.is_mobile column")?;
            }
            tx.execute(
                "UPDATE schema_version SET version = ?1 WHERE id = 1",
                params![6],
            )?;
            Ok(())
        })
    }

    /// v6 -> v7: add the nullable quality bundle to history observations.
    /// Existing rows intentionally remain all-NULL (legacy unknown). Each
    /// column is probed independently so an interrupted/manual partial repair
    /// can be completed without duplicate-column failures. A same-named
    /// column with a different declaration is corruption, not a shape we can
    /// safely reinterpret.
    fn migrate_v6_to_v7(&self) -> Result<()> {
        const COLUMNS: [(&str, &str); 5] = [
            ("implied_speed_kmh", "REAL"),
            ("last_fix_age_secs", "INTEGER"),
            ("inlier_ratio", "REAL"),
            ("confidence", "TEXT"),
            ("suspect_reason", "TEXT"),
        ];

        self.run_migration(|tx| {
            let mut existing = std::collections::HashMap::new();
            {
                let mut statement = tx.prepare("PRAGMA table_info(fixes)")?;
                let rows = statement.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })?;
                for row in rows {
                    let (name, declared_type, not_null, default_value) = row?;
                    existing.insert(name, (declared_type, not_null, default_value));
                }
            }

            for (name, expected_type) in COLUMNS {
                if let Some((declared_type, not_null, default_value)) = existing.get(name) {
                    if declared_type != expected_type || *not_null != 0 || default_value.is_some() {
                        anyhow::bail!(
                            "malformed fixes.{name}: expected {expected_type} NULL with no default, \
                             found type={declared_type:?} notnull={not_null} default={default_value:?}"
                        );
                    }
                } else {
                    tx.execute(
                        &format!("ALTER TABLE fixes ADD COLUMN {name} {expected_type}"),
                        [],
                    )
                    .with_context(|| format!("failed to add fixes.{name} column"))?;
                }
            }

            tx.execute(
                "UPDATE schema_version SET version = ?1 WHERE id = 1",
                params![7],
            )?;
            Ok(())
        })
    }

    /// v7 -> v8: rename the WiGLE daily-quota counter metadata key from the
    /// generic `api_calls_today` to the provider-specific `wigle_lookups_today`
    /// (task-0130). Only WiGLE is metered (Apple is unmetered), so the generic
    /// name mis-described a WiGLE-specific counter.
    ///
    /// LOSSLESS + IDEMPOTENT-SAFE: this is a pure key rename that PRESERVES the
    /// value, so a daemon with a live same-day counter keeps its count across
    /// the upgrade. If the old key is absent (no lookups spent yet) the UPDATE
    /// matches zero rows — a no-op — and the counter simply starts fresh under
    /// the new key. The paired `api_calls_date` reset key is intentionally left
    /// untouched: it is internal, never exposed on the wire, and renaming it is
    /// out of scope for this task. Re-running is safe: once renamed there is no
    /// `api_calls_today` row left to match.
    fn migrate_v7_to_v8(&self) -> Result<()> {
        self.run_migration(|tx| {
            tx.execute(
                "UPDATE metadata SET key = 'wigle_lookups_today' WHERE key = 'api_calls_today'",
                [],
            )
            .context("failed to rename api_calls_today metadata key")?;
            tx.execute(
                "UPDATE schema_version SET version = ?1 WHERE id = 1",
                params![8],
            )?;
            Ok(())
        })
    }

    /// v8 -> v9: DROP the redundant `channel` column from `aps` and `pending`
    /// (task-0123).
    ///
    /// `(band, channel)` is a pure function of `frequency`, which both tables
    /// already store, so the bare `channel` number carried no information the
    /// frequency did not — and, worse, was AMBIGUOUS once Wi-Fi 6E/7 made 6 GHz
    /// reuse the low channel numbers (ch1@2412MHz != ch1@5955MHz). Frequency is
    /// canonical; consumers derive the band-qualified
    /// `whereami_protocol::Channel` on read. This mirrors task-0122's
    /// compute-on-read / single-source-of-truth philosophy.
    ///
    /// LOSSLESS for the data that matters: only the derived, redundant column is
    /// removed; `frequency` and every other column survive untouched. A v8 DB
    /// with live rows opens cleanly at v9 with its frequencies intact.
    ///
    /// Mechanism: `ALTER TABLE ... DROP COLUMN`, supported by SQLite >= 3.35
    /// (2021); rusqlite's bundled SQLite is newer. Each drop is guarded by a
    /// `PRAGMA table_info` probe so the migration is idempotent and a no-op on
    /// a table that never had the column (e.g. a re-run, or a hypothetical fresh
    /// shape that already omitted it). The drop and the version stamp share one
    /// `run_migration` transaction, so a crash mid-migration rolls back to v8.
    fn migrate_v8_to_v9(&self) -> Result<()> {
        self.run_migration(|tx| {
            let has_channel = |table: &str| -> Result<bool> {
                let mut stmt = tx.prepare(&format!("PRAGMA table_info({table})"))?;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    if row.get::<_, String>(1)? == "channel" {
                        return Ok(true);
                    }
                }
                Ok(false)
            };
            for table in ["aps", "pending"] {
                if has_channel(table)? {
                    tx.execute(&format!("ALTER TABLE {table} DROP COLUMN channel"), [])
                        .with_context(|| format!("failed to drop {table}.channel column"))?;
                }
            }
            tx.execute(
                "UPDATE schema_version SET version = ?1 WHERE id = 1",
                params![9],
            )?;
            Ok(())
        })
    }

    fn get_schema_version(&self) -> i32 {
        // Table may not exist yet
        let result: Result<i32, _> =
            self.conn
                .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                    row.get(0)
                });
        result.unwrap_or(0)
    }

    /// Does a table named `name` exist? Used by the shape-repair in migrate()
    /// (task-0191). A query error (e.g. a locked or corrupt catalog) is
    /// treated as "absent" — the conservative choice, since the migration
    /// chain that follows re-creates the table idempotently anyway.
    fn table_exists(&self, name: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                params![name],
                |_| Ok(()),
            )
            .optional()
            .map(|o| o.is_some())
            .unwrap_or(false)
    }

    /// task-0191 AC#3: the highest schema version whose on-disk *table shape*
    /// is actually present, capped at the `stamped` version (we never claim a
    /// shape newer than the stored stamp). v4 introduced `last_fix`, v5
    /// introduced `fixes`, v6 introduced `aps.is_mobile`, and v7 introduced
    /// the complete five-column history quality bundle. Each layer
    /// depends on the previous one, so a missing earlier shape caps the
    /// satisfied version before it. Versions below 4 touch only tables
    /// create_schema_v1 always creates, so anything < 4 is shape-satisfied by
    /// definition and returned unchanged (no sqlite_master probe needed).
    fn highest_satisfied_shape_version(&self, stamped: i32) -> i32 {
        if stamped >= 4 && !self.table_exists("last_fix") {
            return 3;
        }
        if stamped >= 5 && !self.table_exists("fixes") {
            return 4;
        }
        if stamped >= 6 && !self.column_exists("aps", "is_mobile") {
            return 5;
        }
        if stamped >= 7 && !self.fix_quality_columns_have_expected_shape() {
            return 6;
        }
        stamped
    }

    fn fix_quality_columns_have_expected_shape(&self) -> bool {
        const EXPECTED: [(&str, &str); 5] = [
            ("implied_speed_kmh", "REAL"),
            ("last_fix_age_secs", "INTEGER"),
            ("inlier_ratio", "REAL"),
            ("confidence", "TEXT"),
            ("suspect_reason", "TEXT"),
        ];
        let Ok(mut statement) = self.conn.prepare("PRAGMA table_info(fixes)") else {
            return false;
        };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        }) else {
            return false;
        };
        let mut columns = std::collections::HashMap::new();
        for row in rows {
            let Ok((name, declared_type, not_null, default_value)) = row else {
                return false;
            };
            columns.insert(name, (declared_type, not_null, default_value));
        }
        EXPECTED.iter().all(|(name, expected_type)| {
            columns
                .get(*name)
                .is_some_and(|(declared_type, not_null, default_value)| {
                    declared_type == expected_type && *not_null == 0 && default_value.is_none()
                })
        })
    }

    fn column_exists(&self, table: &str, column: &str) -> bool {
        let Ok(mut stmt) = self.conn.prepare(&format!("PRAGMA table_info({table})")) else {
            return false;
        };
        let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
            return false;
        };
        for name in rows {
            if name.is_ok_and(|name| name == column) {
                return true;
            }
        }
        false
    }

    /// Create the genuine v1 shape. This is the FROZEN historical v1 DDL, NOT
    /// the current shape: a fresh DB runs this and then the full migration
    /// cascade (e.g. v8->v9 drops `aps`/`pending.channel`). The `channel`
    /// columns are deliberately RETAINED here even though v9 removes them —
    /// v1 authentically had them, and `build_historical_shape` reconstructs real
    /// pre-v9 databases solely from this DDL plus the real migrate fns. Editing
    /// v1 to omit `channel` would make those historical shapes lie and would rob
    /// the v8->v9 drop test of a real column to drop (task-0123).
    fn create_schema_v1(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS aps (
                bssid       TEXT PRIMARY KEY,
                ssid        TEXT,
                lat         REAL NOT NULL,
                lon         REAL NOT NULL,
                encryption  TEXT,
                channel     INTEGER,
                frequency   INTEGER,
                city        TEXT,
                country     TEXT,
                source      TEXT NOT NULL,
                first_seen  TEXT NOT NULL,
                last_seen   TEXT NOT NULL,
                fetched_at  TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_aps_geo ON aps(lat, lon);
            CREATE INDEX IF NOT EXISTS idx_aps_last_seen ON aps(last_seen);

            CREATE TABLE IF NOT EXISTS not_found (
                bssid       TEXT PRIMARY KEY,
                first_seen  TEXT NOT NULL,
                last_seen   TEXT NOT NULL,
                checked_at  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pending (
                bssid       TEXT PRIMARY KEY,
                ssid        TEXT,
                channel     INTEGER,
                frequency   INTEGER,
                signal_dbm  INTEGER,
                first_seen  TEXT NOT NULL,
                last_seen   TEXT NOT NULL,
                attempts    INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS metadata (
                key         TEXT PRIMARY KEY,
                value       TEXT NOT NULL
            );
        ",
            )
            .context("failed to create schema v1")?;

        // Set version to 1 (this function only creates the v1 shape).
        // Subsequent migrate_v1_to_v2 / migrate_v2_to_v3 will bump it.
        // We previously inserted SCHEMA_VERSION here, which short-circuited
        // newer migrations on a fresh database.
        let count: i32 = self
            .conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))?;
        if count == 0 {
            self.conn
                .execute("INSERT INTO schema_version (version) VALUES (1)", [])?;
        }

        Ok(())
    }

    // --- aps table ---

    /// Look up an AP by BSSID. Returns None if not cached.
    pub fn get_ap(&self, bssid: &str) -> Result<Option<ApInfo>> {
        let result = self
            .conn
            .query_row(
                "SELECT bssid, ssid, lat, lon, encryption, frequency, city, country, source, is_mobile
                 FROM aps WHERE bssid = ?1",
                params![bssid],
                |row| {
                    let source_str: String = row.get(8)?;
                    Ok(ApInfo {
                        bssid: row.get(0)?,
                        ssid: row.get(1)?,
                        lat: row.get(2)?,
                        lon: row.get(3)?,
                        encryption: row.get(4)?,
                        frequency: row.get(5)?,
                        city: row.get(6)?,
                        country: row.get(7)?,
                        is_mobile: row.get::<_, i64>(9)? != 0,
                        source: Source::from_db_str(&source_str),
                    })
                },
            )
            .optional()?;
        Ok(result)
    }

    /// Insert or update a resolved AP.
    ///
    /// Source priority is enforced for provider-owned position metadata: an
    /// existing row is overwritten only if
    /// the incoming row's source priority is **strictly greater than or
    /// equal to** the stored row's. This keeps better fixes (Apple) sticky
    /// against later, lower-quality writes (WiGLE/BeaconDB) for the same
    /// BSSID.
    ///
    /// Scanner-owned `ssid`, derived `is_mobile`, and `last_seen` are always
    /// refreshed regardless of provider priority. Observation metadata is
    /// independent of whose remote position is authoritative.
    #[cfg(test)]
    pub fn upsert_ap(&self, ap: &ApInfo) -> Result<()> {
        Self::upsert_ap_on(&self.conn, ap, &self.mobile_ssid_denylist)
    }

    fn upsert_ap_on(conn: &Connection, ap: &ApInfo, mobile_ssid_denylist: &[String]) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let source_str = ap.source.as_str();
        let priority = ap.source.priority();
        let is_mobile = i64::from(crate::scanner::is_mobile_ssid(
            ap.ssid.as_deref(),
            mobile_ssid_denylist,
        ));
        conn.execute(
            "INSERT INTO aps (bssid, ssid, lat, lon, encryption, frequency, city, country, source, source_priority, is_mobile, first_seen, last_seen, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, ?12)
             ON CONFLICT(bssid) DO UPDATE SET
                ssid       = excluded.ssid,
                is_mobile  = excluded.is_mobile,
                lat        = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.lat        ELSE aps.lat        END,
                lon        = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.lon        ELSE aps.lon        END,
                encryption = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.encryption ELSE aps.encryption END,
                frequency  = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.frequency  ELSE aps.frequency  END,
                city       = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.city       ELSE aps.city       END,
                country    = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.country    ELSE aps.country    END,
                source           = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.source           ELSE aps.source           END,
                source_priority  = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.source_priority  ELSE aps.source_priority  END,
                fetched_at       = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.fetched_at       ELSE aps.fetched_at       END,
                last_seen  = excluded.last_seen",
            params![ap.bssid, ap.ssid, ap.lat, ap.lon, ap.encryption, ap.frequency, ap.city, ap.country, source_str, priority, is_mobile, now],
        )?;
        Ok(())
    }

    fn cache_resolved_ap(&mut self, ap: &ApInfo, delete_pending: bool) -> Result<()> {
        let mobile_ssid_denylist = &self.mobile_ssid_denylist;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::upsert_ap_on(&tx, ap, mobile_ssid_denylist)?;
        if delete_pending {
            tx.execute("DELETE FROM pending WHERE bssid = ?1", params![ap.bssid])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Return every cached AP in input order and advance all of their
    /// `last_seen` values in one IMMEDIATE transaction. A locate with ten
    /// cache hits therefore performs one commit instead of ten autocommits.
    fn lookup_cached_touch(&mut self, bssids: &[String]) -> Result<Vec<ApInfo>> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        let mut result = Vec::new();
        for bssid in bssids {
            let ap = tx
                .query_row(
                    "SELECT bssid, ssid, lat, lon, encryption, frequency, city, country, source, is_mobile
                     FROM aps WHERE bssid = ?1",
                    params![bssid],
                    |row| {
                        let source_str: String = row.get(8)?;
                        Ok(ApInfo {
                            bssid: row.get(0)?,
                            ssid: row.get(1)?,
                            lat: row.get(2)?,
                            lon: row.get(3)?,
                            encryption: row.get(4)?,
                            frequency: row.get(5)?,
                            city: row.get(6)?,
                            country: row.get(7)?,
                            is_mobile: row.get::<_, i64>(9)? != 0,
                            source: Source::from_db_str(&source_str),
                        })
                    },
                )
                .optional()?;
            if let Some(ap) = ap {
                tx.execute(
                    "UPDATE aps SET last_seen = ?1 WHERE bssid = ?2",
                    params![now, bssid],
                )?;
                result.push(ap);
            }
        }
        tx.commit()?;
        Ok(result)
    }

    /// Return every cached AP in input order under one read snapshot, without
    /// updating observation metadata.
    fn lookup_cached_readonly(&mut self, bssids: &[String]) -> Result<Vec<ApInfo>> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let mut result = Vec::new();
        for bssid in bssids {
            let ap = tx
                .query_row(
                    "SELECT bssid, ssid, lat, lon, encryption, frequency, city, country, source, is_mobile
                     FROM aps WHERE bssid = ?1",
                    params![bssid],
                    |row| {
                        let source_str: String = row.get(8)?;
                        Ok(ApInfo {
                            bssid: row.get(0)?,
                            ssid: row.get(1)?,
                            lat: row.get(2)?,
                            lon: row.get(3)?,
                            encryption: row.get(4)?,
                            frequency: row.get(5)?,
                            city: row.get(6)?,
                            country: row.get(7)?,
                            is_mobile: row.get::<_, i64>(9)? != 0,
                            source: Source::from_db_str(&source_str),
                        })
                    },
                )
                .optional()?;
            if let Some(ap) = ap {
                result.push(ap);
            }
        }
        tx.commit()?;
        Ok(result)
    }

    // --- not_found table ---

    /// Check if a BSSID is in the not_found table and not expired.
    pub fn is_not_found(&self, bssid: &str, ttl_days: i64) -> Result<bool> {
        let cutoff = (Utc::now() - chrono::TimeDelta::days(ttl_days)).to_rfc3339();
        let count: i32 = self.conn.query_row(
            "SELECT COUNT(*) FROM not_found WHERE bssid = ?1 AND checked_at > ?2",
            params![bssid, cutoff],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Insert into not_found.
    #[cfg(test)]
    pub fn insert_not_found(&self, bssid: &str) -> Result<()> {
        Self::insert_not_found_on(&self.conn, bssid)
    }

    fn insert_not_found_on(conn: &Connection, bssid: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO not_found (bssid, first_seen, last_seen, checked_at)
             VALUES (?1, ?2, ?2, ?2)
             ON CONFLICT(bssid) DO UPDATE SET last_seen = excluded.last_seen, checked_at = excluded.checked_at",
            params![bssid, now],
        )?;
        Ok(())
    }

    fn record_not_found(&mut self, bssid: &str, delete_pending: bool) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::insert_not_found_on(&tx, bssid)?;
        if delete_pending {
            tx.execute("DELETE FROM pending WHERE bssid = ?1", params![bssid])?;
        }
        tx.commit()?;
        Ok(())
    }

    // --- pending table ---

    /// Insert a BSSID into the pending queue.
    #[cfg(test)]
    pub fn insert_pending(
        &self,
        bssid: &str,
        ssid: Option<&str>,
        frequency: Option<i32>,
        signal_dbm: Option<i32>,
    ) -> Result<()> {
        Self::insert_pending_on(&self.conn, bssid, ssid, frequency, signal_dbm)
    }

    fn insert_pending_on(
        conn: &Connection,
        bssid: &str,
        ssid: Option<&str>,
        frequency: Option<i32>,
        signal_dbm: Option<i32>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        // pending.channel was dropped at schema v9 (task-0123): frequency is the
        // canonical physical identifier, so the redundant band-less number is no
        // longer stored.
        conn.execute(
            "INSERT INTO pending (bssid, ssid, frequency, signal_dbm, first_seen, last_seen, attempts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, 0)
             ON CONFLICT(bssid) DO UPDATE SET
                ssid = excluded.ssid,
                last_seen = excluded.last_seen,
                signal_dbm = CASE WHEN excluded.signal_dbm > pending.signal_dbm THEN excluded.signal_dbm ELSE pending.signal_dbm END",
            params![bssid, ssid, frequency, signal_dbm, now],
        )?;
        Ok(())
    }

    fn insert_pending_many(&mut self, observations: &[(String, Option<String>)]) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (bssid, ssid) in observations {
            Self::insert_pending_on(&tx, bssid, ssid.as_deref(), None, None)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Get up to `limit` pending observations, ordered by fewest attempts.
    /// SSID is a production input: pending retries must classify the scanner
    /// value captured before the original provider request.
    pub fn get_pending(&self, limit: usize) -> Result<Vec<PendingAp>> {
        let mut stmt = self.conn.prepare(
            "SELECT bssid, ssid, attempts
             FROM pending ORDER BY attempts ASC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(PendingAp {
                bssid: row.get(0)?,
                ssid: row.get(1)?,
                attempts: row.get(2)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Cheap existence check: is this BSSID currently in the pending queue?
    ///
    /// Uses an indexed PRIMARY KEY probe. Prefer this over
    /// `get_pending(N).iter().any(...)` — that pattern is O(N) per call and
    /// scans rows we never inspect. For per-BSSID checks (debug rendering,
    /// not_found end-of-chain logic) this is the right primitive.
    pub fn is_pending(&self, bssid: &str) -> Result<bool> {
        let exists: Option<i32> = self
            .conn
            .query_row(
                "SELECT 1 FROM pending WHERE bssid = ?1 LIMIT 1",
                params![bssid],
                |row| row.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }

    /// Increment attempts for a pending BSSID.
    #[cfg(test)]
    pub fn increment_pending_attempts(&self, bssid: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE pending SET attempts = attempts + 1 WHERE bssid = ?1",
            params![bssid],
        )?;
        Ok(())
    }

    /// Delete a BSSID from the pending table.
    #[cfg(test)]
    pub fn delete_pending(&self, bssid: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM pending WHERE bssid = ?1", params![bssid])?;
        Ok(())
    }

    /// Delete pending entries that have exceeded max attempts.
    pub fn delete_expired_pending(&self, max_attempts: i32) -> Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM pending WHERE attempts >= ?1",
            params![max_attempts],
        )?;
        Ok(deleted)
    }

    /// Prune not_found rows whose `checked_at` is older than `2 × ttl_days`.
    ///
    /// Past 1×ttl a row is already never re-checked (`is_not_found` only
    /// counts rows within ttl, and `revive_expired_not_found` moves expired
    /// rows back to pending in bounded batches). 2×ttl is therefore a safe
    /// soft cap on definitively-dead rows, bounding the monotonic growth of
    /// the negative cache on a long-running daemon at a busy location
    /// (task-0110). The cutoff mirrors `is_not_found`'s time arithmetic so the
    /// boundary is consistent. Returns the number of rows deleted.
    pub fn prune_not_found(&self, ttl_days: i64) -> Result<usize> {
        let cutoff = (Utc::now() - chrono::TimeDelta::days(2 * ttl_days)).to_rfc3339();
        let deleted = self.conn.execute(
            "DELETE FROM not_found WHERE checked_at < ?1",
            params![cutoff],
        )?;
        Ok(deleted)
    }

    // --- metadata (rate limiting) ---

    /// Get a metadata value by key.
    pub fn get_metadata(&self, key: &str) -> Result<Option<String>> {
        let result = self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(result)
    }

    /// Set (insert or overwrite) a metadata value by key.
    pub fn set_metadata(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    #[cfg(test)]
    // The library target omits provider.rs, whose binary-target tests use this
    // helper to induce a genuine SQLite reservation failure.
    #[allow(dead_code)]
    pub(crate) fn drop_metadata_table_for_test(&self) -> Result<()> {
        self.conn.execute_batch("DROP TABLE metadata")?;
        Ok(())
    }

    /// Atomically reserve an API call slot for today.
    ///
    /// Performs daily-counter reset, current-count read, and increment in a
    /// single IMMEDIATE transaction. Distinguishes a committed reservation,
    /// normal daily exhaustion, and fail-closed persisted-counter corruption.
    ///
    /// On any transient failure of the network call afterwards, the caller
    /// SHOULD call [`Database::refund_wigle_lookup`] to give the slot back.
    /// Slots that map to actual completed lookups (Found / NotFound) should
    /// not be refunded.
    ///
    /// We optimistically mirror WiGLE's server-side daily counter; drift is
    /// possible if other clients share the same WiGLE credentials, so this
    /// local ceiling is a best-effort guard, not an authoritative balance.
    #[allow(dead_code)]
    pub fn try_reserve_wigle_lookup(&mut self, daily_limit: u32) -> Result<QuotaDecision> {
        #[cfg(test)]
        let result = self.try_reserve_wigle_lookup_inner(daily_limit, None);
        #[cfg(not(test))]
        let result = self.try_reserve_wigle_lookup_inner(daily_limit);
        result
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn try_reserve_wigle_lookup_observing<F>(
        &mut self,
        daily_limit: u32,
        mut observe_transaction: F,
    ) -> Result<QuotaDecision>
    where
        F: FnMut(rusqlite::TransactionState),
    {
        self.try_reserve_wigle_lookup_inner(daily_limit, Some(&mut observe_transaction))
    }

    fn try_reserve_wigle_lookup_inner(
        &mut self,
        daily_limit: u32,
        #[cfg(test)] observe_transaction: Option<&mut dyn FnMut(rusqlite::TransactionState)>,
    ) -> Result<QuotaDecision> {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        #[cfg(test)]
        if let Some(observe_transaction) = observe_transaction {
            observe_transaction(tx.transaction_state(None)?);
        }

        // Reset counter if the stored date is not today.
        let stored_date: Option<String> = tx
            .query_row(
                "SELECT value FROM metadata WHERE key = 'api_calls_date'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let count: u32 = if stored_date.as_deref() == Some(&today) {
            let raw: Option<String> = tx
                .query_row(
                    "SELECT value FROM metadata WHERE key = 'wigle_lookups_today'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            match parse_wigle_lookups_value(raw) {
                Ok(v) => v,
                // task-0097: corrupt counter -> fail closed. Deny the slot and
                // return WITHOUT writing. Healing the garbage here would reset
                // the counter and re-charge up to `daily_limit` fresh WiGLE
                // calls on top of whatever was already spent. Dropping `tx`
                // uncommitted rolls back cleanly (no write has happened on this
                // branch). Parsing is side-effect-free; this reservation
                // context owns the precise warning.
                Err(corrupt) => {
                    tracing::warn!(
                        "{}; WiGLE quota reservation denied to avoid \
                         double-spending the daily quota",
                        corrupt
                    );
                    return Ok(QuotaDecision::CounterCorrupt);
                }
            }
        } else {
            tx.execute(
                "INSERT INTO metadata (key, value) VALUES ('api_calls_date', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![&today],
            )?;
            0
        };

        if count >= daily_limit {
            // Persist the date reset (if any) without bumping the counter.
            tx.execute(
                "INSERT INTO metadata (key, value) VALUES ('wigle_lookups_today', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![count.to_string()],
            )?;
            tx.commit()?;
            return Ok(QuotaDecision::Exhausted);
        }

        let new_count = count + 1;
        tx.execute(
            "INSERT INTO metadata (key, value) VALUES ('wigle_lookups_today', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![new_count.to_string()],
        )?;
        tx.commit()?;
        Ok(QuotaDecision::Reserved)
    }

    /// Refund a previously reserved WiGLE lookup slot. Clamped at 0. Only
    /// meaningful when called on the same calendar day; if the day has rolled
    /// over the refund is silently ignored (counter has already been reset to
    /// 0). We optimistically mirror WiGLE's server-side daily counter; drift is
    /// possible if other clients share the same WiGLE credentials.
    #[allow(dead_code)]
    pub fn refund_wigle_lookup(&mut self) -> Result<()> {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored_date: Option<String> = tx
            .query_row(
                "SELECT value FROM metadata WHERE key = 'api_calls_date'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if stored_date.as_deref() != Some(&today) {
            // Day rolled over since reservation; counter already reset.
            tx.commit()?;
            return Ok(());
        }
        let raw: Option<String> = tx
            .query_row(
                "SELECT value FROM metadata WHERE key = 'wigle_lookups_today'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        // task-0097: a corrupt counter propagates as Err (tx rolls back, no
        // write). Refunding by healing the garbage to "0" would re-introduce
        // the double-spend heal this task closes, so fail closed here too.
        let count = match parse_wigle_lookups_value(raw) {
            Ok(count) => count,
            Err(corrupt) => {
                tracing::warn!(
                    "{}; WiGLE quota refund skipped so the corrupt persisted \
                     counter remains untouched",
                    corrupt
                );
                return Err(corrupt.into());
            }
        };
        let new_count = count.saturating_sub(1);
        tx.execute(
            "INSERT INTO metadata (key, value) VALUES ('wigle_lookups_today', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![new_count.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Get the number of WiGLE lookups spent today.
    #[cfg(test)]
    pub fn wigle_lookups_today(&self) -> Result<u32> {
        parse_wigle_lookups_value(self.get_metadata("wigle_lookups_today")?).map_err(Into::into)
    }

    fn wigle_lookups_today_for_stats(&self) -> Result<u32> {
        let raw = self.get_metadata("wigle_lookups_today")?;
        match parse_wigle_lookups_value(raw) {
            Ok(count) => Ok(count),
            Err(corrupt) => {
                tracing::warn!(
                    "{}; stats reports wigle_lookups_today fallback 0 while the \
                     persisted counter remains untouched",
                    corrupt
                );
                Ok(0)
            }
        }
    }

    // --- stats ---

    pub fn cached_ap_count(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM aps", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn pending_ap_count(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM pending", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn not_found_ap_count(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM not_found", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn db_size_bytes(&self) -> Result<i64> {
        let page_count: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let page_size: i64 = self
            .conn
            .query_row("PRAGMA page_size", [], |row| row.get(0))?;
        Ok(page_count * page_size)
    }

    /// Atomically move a bounded batch of expired negative-cache entries back
    /// into pending. This replaces a read followed by two autocommits per row.
    fn revive_expired_not_found(&mut self, ttl_days: i64, limit: usize) -> Result<Vec<String>> {
        let cutoff = (Utc::now() - chrono::TimeDelta::days(ttl_days)).to_rfc3339();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let expired = {
            let mut stmt =
                tx.prepare("SELECT bssid FROM not_found WHERE checked_at <= ?1 LIMIT ?2")?;
            let rows = stmt.query_map(params![cutoff, limit as i64], |row| row.get(0))?;
            let mut values = Vec::new();
            for row in rows {
                values.push(row?);
            }
            values
        };
        let now = Utc::now().to_rfc3339();
        for bssid in &expired {
            tx.execute("DELETE FROM not_found WHERE bssid = ?1", params![bssid])?;
            tx.execute(
                "INSERT INTO pending (bssid, first_seen, last_seen, attempts)
                 VALUES (?1, ?2, ?2, 0)
                 ON CONFLICT(bssid) DO UPDATE SET last_seen = excluded.last_seen",
                params![bssid, now],
            )?;
        }
        tx.commit()?;
        Ok(expired)
    }

    // --- last_fix table ---

    /// Persist the last-known position into the single-row `last_fix` table.
    ///
    /// **Invariant: monotonic by `at_rfc3339` — older writes are rejected and
    /// warned after the transaction commits.** The UPSERT overwrites the
    /// existing row only when the incoming `at_rfc3339` is `>=` the stored one
    /// (see the trailing `WHERE`). Concurrent locates are serialized by the
    /// daemon's `last_fix` guard and writer FIFO (task-0046); this guard instead
    /// defends against the wall clock moving backwards (NTP step, container
    /// time fix on restart) letting a stale fix clobber a newer one (task-0100).
    ///
    /// The string `>=` is a valid chronological test only because every
    /// writer produces `DateTime::<Utc>::to_rfc3339()`: a fixed-width
    /// `YYYY-MM-DDTHH:MM:SS` prefix and a constant `+00:00` offset, so
    /// lexicographic order matches time order. (AutoSi variable-width
    /// fractional seconds still sort correctly because the `.` that starts a
    /// fraction sorts after the `+` of the offset.) A non-UTC offset, or a
    /// writer switching timestamp formats, would break this.
    pub fn set_last_fix(&self, fix: &LastFix) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let outcome = upsert_last_fix(&tx, fix)?;
        tx.commit()?;
        warn_if_last_fix_dropped(outcome);
        Ok(())
    }

    // --- fixes table (timeseries; task-0031) ---

    /// Append one fix row to the timeseries.
    ///
    /// task-0100: unlike [`Database::set_last_fix`], the `fixes` timeseries
    /// has **no monotonicity guard**. Rows are ordered by the auto-increment
    /// `id` (insertion order), not by `at_rfc3339`, so a wall-clock skew can
    /// append a row whose timestamp predates its neighbour. That is
    /// intentional: a timeseries records observations in the order they
    /// happened. `get_fixes_in_range` still filters and sorts by `at_rfc3339`,
    /// so an out-of-order row lands in its correct time bucket. No DB-side
    /// `CHECK` is added — it would require a schema migration and would reject
    /// legitimate skew-affected observations.
    #[cfg(test)]
    pub fn insert_fix(
        &self,
        at_rfc3339: &str,
        lat: f64,
        lon: f64,
        accuracy_m: f64,
        n_sources: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO fixes (at_rfc3339, lat, lon, accuracy_m, n_sources)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![at_rfc3339, lat, lon, accuracy_m, n_sources],
        )?;
        Ok(())
    }

    /// Persist the current fix and its history observation in one transaction.
    /// The monotonic guard in `set_last_fix` remains in force.
    fn persist_fix_and_history(&mut self, fix: &LastFix, quality: &FixQuality) -> Result<()> {
        let outcome = self.persist_fix_and_history_outcome(fix, quality)?;
        warn_if_last_fix_dropped(outcome);
        Ok(())
    }

    fn persist_fix_and_history_outcome(
        &mut self,
        fix: &LastFix,
        quality: &FixQuality,
    ) -> Result<LastFixUpdate> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let outcome = upsert_last_fix(&tx, fix)?;
        // Same RFC3339 marshalling as the UPSERT (invariant in `upsert_last_fix`):
        // the history row's `at_rfc3339` string is what `get_fixes_in_range`'s
        // lexical bounds order by.
        tx.execute(
            "INSERT INTO fixes (
                 at_rfc3339, lat, lon, accuracy_m, n_sources,
                 implied_speed_kmh, last_fix_age_secs, inlier_ratio, confidence, suspect_reason
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                fix.at.to_rfc3339(),
                fix.lat,
                fix.lon,
                fix.accuracy_m,
                fix.sources as i64,
                quality.implied_speed_kmh(),
                quality.last_fix_age_secs(),
                quality.inlier_ratio(),
                quality.confidence().as_db_str(),
                quality.suspect_reason(),
            ],
        )?;
        tx.commit()?;
        Ok(outcome)
    }

    /// Remove all persisted location and observed-network state atomically.
    /// Schema and operational metadata (quota + shutdown observability) remain.
    fn wipe_privacy_data(&mut self) -> Result<PrivacyWipeCounts> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let counts = PrivacyWipeCounts {
            aps_deleted: tx.execute("DELETE FROM aps", [])?,
            pending_deleted: tx.execute("DELETE FROM pending", [])?,
            not_found_deleted: tx.execute("DELETE FROM not_found", [])?,
            fixes_deleted: tx.execute("DELETE FROM fixes", [])?,
            last_fix_deleted: tx.execute("DELETE FROM last_fix", [])?,
        };
        // AUTOINCREMENT stores the historical high-water mark separately from
        // fixes. It carries no coordinates, but retaining the prior count would
        // contradict "delete all history" and leaks how many fixes once existed.
        tx.execute("DELETE FROM sqlite_sequence WHERE name = 'fixes'", [])?;
        tx.commit()?;
        Ok(counts)
    }

    /// Read fix rows whose `at_rfc3339` falls within `[start_rfc3339, end_rfc3339]`,
    /// in ascending time order.
    pub fn get_fixes_in_range(&self, start_rfc3339: &str, end_rfc3339: &str) -> Result<Vec<Fix>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, at_rfc3339, lat, lon, accuracy_m, n_sources,
                    implied_speed_kmh, last_fix_age_secs, inlier_ratio, confidence, suspect_reason
             FROM fixes
             WHERE at_rfc3339 >= ?1 AND at_rfc3339 <= ?2
             ORDER BY at_rfc3339 ASC",
        )?;
        let rows = stmt.query_map(params![start_rfc3339, end_rfc3339], |r| {
            let id = r.get(0)?;
            let at_rfc3339: String = r.get(1)?;
            Ok(Fix {
                at: parse_fix_timestamp(id, &at_rfc3339)?,
                lat: r.get(2)?,
                lon: r.get(3)?,
                accuracy_m: r.get(4)?,
                n_sources: r.get(5)?,
                quality: decode_fix_quality(
                    id,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                )?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Count the inclusive range and select its newest `max_fixes` rows from
    /// one deferred read transaction. The result is reversed after collection
    /// so callers receive ascending `(timestamp, id)` order while SQLite never
    /// materializes older rows outside the requested page.
    pub(crate) fn get_capped_fixes_in_range(
        &self,
        start_rfc3339: &str,
        end_rfc3339: &str,
        max_fixes: u64,
    ) -> Result<CappedFixRows> {
        self.get_capped_fixes_in_range_with_hook(start_rfc3339, end_rfc3339, max_fixes, || {})
    }

    fn get_capped_fixes_in_range_with_hook<H>(
        &self,
        start_rfc3339: &str,
        end_rfc3339: &str,
        max_fixes: u64,
        after_count: H,
    ) -> Result<CappedFixRows>
    where
        H: FnOnce(),
    {
        if max_fixes == 0 {
            anyhow::bail!("raw history max_fixes must be positive");
        }
        let limit_i64 = i64::try_from(max_fixes)
            .context("raw history max_fixes does not fit SQLite INTEGER")?;
        let _limit_usize = usize::try_from(max_fixes)
            .context("raw history max_fixes does not fit this platform's address space")?;

        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Deferred)?;
        let total: i64 = tx.query_row(
            COUNT_FIXES_IN_RANGE_SQL,
            params![start_rfc3339, end_rfc3339],
            |row| row.get(0),
        )?;
        after_count();

        let mut statement = tx.prepare(SELECT_NEWEST_FIXES_IN_RANGE_SQL)?;
        let rows = statement.query_map(params![start_rfc3339, end_rfc3339, limit_i64], |row| {
            let id = row.get(0)?;
            let at_rfc3339: String = row.get(1)?;
            Ok(Fix {
                at: parse_fix_timestamp(id, &at_rfc3339)?,
                lat: row.get(2)?,
                lon: row.get(3)?,
                accuracy_m: row.get(4)?,
                n_sources: row.get(5)?,
                quality: decode_fix_quality(
                    id,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                )?,
            })
        })?;
        // Deliberately avoid with_capacity(max_fixes): even validated protocol
        // input is not a trustworthy allocation size at this lower boundary.
        let mut fixes = Vec::new();
        for row in rows {
            fixes.push(row?);
        }
        drop(statement);
        tx.commit()?;
        fixes.reverse();

        let returned = i64::try_from(fixes.len())
            .context("raw history result length does not fit SQLite INTEGER")?;
        let omitted = total
            .checked_sub(returned)
            .ok_or_else(|| anyhow::anyhow!("raw history count was smaller than selected page"))?;
        Ok(CappedFixRows {
            fixes,
            elided: (omitted > 0).then_some(omitted),
        })
    }

    /// Delete fixes older than `retention_days`. Returns the number deleted.
    ///
    /// task-0101: `retention_days == 0` means "retention disabled — keep
    /// forever" (matching the `--history-retention-days` help text), so it
    /// early-returns `Ok(0)` and deletes nothing. Without this guard the
    /// cutoff would equal `now` and every sweep would wipe the entire
    /// history. Negative and over-policy values are rejected here as well as
    /// at startup, so a future caller that bypasses `Args::validate` still
    /// cannot turn them into a future cutoff and delete live history.
    #[cfg(test)]
    pub fn prune_fixes(&self, retention_days: i64) -> Result<usize> {
        let Some(cutoff) = retention_cutoff(retention_days)? else {
            return Ok(0);
        };
        let n = self
            .conn
            .execute("DELETE FROM fixes WHERE at_rfc3339 < ?1", params![cutoff])?;
        Ok(n)
    }

    fn prune_fixes_chunk(&mut self, cutoff: &str, limit: usize) -> Result<usize> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deleted = tx.execute(PRUNE_FIXES_CHUNK_SQL, params![cutoff, limit as i64])?;
        tx.commit()?;
        Ok(deleted)
    }

    #[cfg(test)]
    pub fn prune_aps(&self, retention_days: i64) -> Result<usize> {
        let Some(cutoff) = retention_cutoff(retention_days)? else {
            return Ok(0);
        };
        let n = self
            .conn
            .execute("DELETE FROM aps WHERE last_seen < ?1", params![cutoff])?;
        Ok(n)
    }

    fn prune_aps_chunk(&mut self, cutoff: &str, limit: usize) -> Result<usize> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deleted = tx.execute(PRUNE_APS_CHUNK_SQL, params![cutoff, limit as i64])?;
        tx.commit()?;
        Ok(deleted)
    }

    /// Read the persisted last-known position, if any.
    ///
    /// task-0134: returns the DOMAIN [`LastFix`] with `at: DateTime<Utc>`. The
    /// Row->domain conversion (RFC3339 parse + task-0238 non-finite guard) runs
    /// in [`last_fix_from_columns`]; a corrupt persisted row is a typed
    /// [`CorruptLastFix`] error (discarded, never served), NOT a silent stale
    /// value. A genuine SQLite read failure is a distinct (non-`CorruptLastFix`)
    /// error so the locate path can branch ignore-vs-abort.
    pub fn get_last_fix(&self) -> Result<Option<LastFix>> {
        let columns = self
            .conn
            .query_row(
                "SELECT lat, lon, accuracy_m, address, at_rfc3339, sources
                 FROM last_fix WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, f64>(0)?,
                        row.get::<_, f64>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?;
        match columns {
            Some((lat, lon, accuracy_m, address, at_rfc3339, sources)) => Ok(Some(
                last_fix_from_columns(lat, lon, accuracy_m, address, at_rfc3339, sources)?,
            )),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::subscriber;

    fn test_quality() -> FixQuality {
        FixQuality::new(None, None, Some(1.0), Confidence::High, None)
            .expect("test quality must be valid")
    }

    /// task-0134: parse a fixed RFC3339 string into the `DateTime<Utc>` that
    /// the domain `LastFix`/`Fix` now carry. `to_rfc3339()` round-trips these
    /// back to the byte-identical `+00:00` form the monotonic-UPSERT and
    /// range-query lexical comparisons rely on.
    fn ts(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .expect("test timestamp must be valid RFC3339")
            .with_timezone(&Utc)
    }

    // task-0255: capture tracing via the shared `test_support::CaptureSink`.
    // This module asserts on (level, message) only, so it drops the unified
    // sink's `target` column. The sync `capture_events` delegates to the shared
    // `capture` helper; the commit-marker tests below need the raw `CaptureSink`
    // (they install a SQLite commit hook that injects a marker BETWEEN sink
    // construction and the scoped install).
    fn to_level_message(
        events: Vec<crate::test_support::CapturedEvent>,
    ) -> Vec<(tracing::Level, String)> {
        events
            .into_iter()
            .map(|(level, _target, message)| (level, message))
            .collect()
    }

    fn capture_events<T>(operation: impl FnOnce() -> T) -> (T, Vec<(tracing::Level, String)>) {
        let (result, events) = crate::test_support::capture(operation);
        (result, to_level_message(events))
    }

    fn warning_messages(events: &[(tracing::Level, String)]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|(level, message)| {
                (*level == tracing::Level::WARN).then_some(message.as_str())
            })
            .collect()
    }

    fn assert_one_last_fix_drop_warning(
        events: &[(tracing::Level, String)],
        incoming: &str,
        stored: &str,
    ) {
        let all_warnings = warning_messages(events);
        assert_eq!(all_warnings.len(), 1, "events: {events:?}");
        let warnings: Vec<_> = all_warnings
            .into_iter()
            .filter(|message| message.contains("persisted last_fix update rejected"))
            .collect();
        assert_eq!(warnings.len(), 1, "events: {events:?}");
        assert!(
            warnings[0].contains(&format!("{incoming:?}")),
            "incoming timestamp missing or unescaped: {}",
            warnings[0]
        );
        assert!(
            warnings[0].contains(&format!("{stored:?}")),
            "stored timestamp missing or unescaped: {}",
            warnings[0]
        );
        assert!(warnings[0].contains("memory/history may still contain the incoming observation"));
    }

    fn install_commit_marker(db: &Database, captured: &crate::test_support::CaptureSink) {
        let sink = captured.clone();
        db.conn.commit_hook(Some(move || {
            // Inject a marker in capture order so the tests can assert the
            // transaction commit precedes the last_fix drop warning. `target`
            // is irrelevant here (the assertions match on the message).
            sink.record_event(tracing::Level::INFO, "test", "test commit");
            false
        }));
    }

    fn assert_commit_precedes_last_fix_warning(events: &[(tracing::Level, String)]) {
        let commit = events
            .iter()
            .position(|(_, message)| message == "test commit")
            .expect("transaction commit marker missing");
        let warning = events
            .iter()
            .position(|(_, message)| message.contains("persisted last_fix update rejected"))
            .expect("last_fix drop warning missing");
        assert!(
            commit < warning,
            "last_fix warning preceded transaction commit: {events:?}"
        );
    }

    fn sample_ap(index: usize) -> ApInfo {
        ApInfo {
            bssid: format!("02:00:00:00:00:{index:02x}"),
            ssid: Some(format!("ap-{index}")),
            lat: 55.67 + index as f64 / 10_000.0,
            lon: 12.56,
            encryption: None,
            frequency: Some(2_412),
            city: None,
            country: None,
            is_mobile: false,
            source: Source::Apple,
        }
    }

    fn seed_api_counter_for_today(db: &Database, raw: &str) {
        db.set_metadata(
            "api_calls_date",
            &chrono::Utc::now().format("%Y-%m-%d").to_string(),
        )
        .unwrap();
        db.set_metadata("wigle_lookups_today", raw).unwrap();
    }

    #[test]
    fn ten_cache_hits_and_fix_persistence_commit_exactly_twice() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let mut db = Database::open_memory().unwrap();
        let bssids: Vec<_> = (0..10)
            .map(|index| {
                let ap = sample_ap(index);
                db.upsert_ap(&ap).unwrap();
                ap.bssid
            })
            .collect();
        let commits = Arc::new(AtomicUsize::new(0));
        let hook_commits = Arc::clone(&commits);
        db.conn.commit_hook(Some(move || {
            hook_commits.fetch_add(1, Ordering::SeqCst);
            false
        }));

        let hits = db.lookup_cached_touch(&bssids).unwrap();
        assert_eq!(hits.len(), 10);
        db.persist_fix_and_history(
            &LastFix {
                lat: 55.6761,
                lon: 12.5683,
                accuracy_m: 8.0,
                address: None,
                at: ts("2026-07-05T00:00:00+00:00"),
                sources: 10,
            },
            &test_quality(),
        )
        .unwrap();

        assert_eq!(
            commits.load(Ordering::SeqCst),
            2,
            "cache touches and last_fix+history must each use one transaction"
        );
    }

    #[test]
    fn prune_chunk_selection_is_index_search_not_full_scan() {
        let db = Database::open_memory().unwrap();
        let explain = format!("EXPLAIN QUERY PLAN {PRUNE_FIXES_CHUNK_SQL}");
        let mut stmt = db.conn.prepare(&explain).unwrap();
        let details: Vec<String> = stmt
            .query_map(params!["2020-01-01T00:00:00+00:00", 1_000_i64], |row| {
                row.get(3)
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            details.iter().any(|detail| {
                detail.contains("SEARCH fixes") && detail.contains("idx_fixes_at")
            }),
            "expired-row selection must SEARCH idx_fixes_at: {details:?}"
        );
        assert!(
            details.iter().all(|detail| !detail.contains("SCAN fixes")),
            "prune plan must never full-scan fixes: {details:?}"
        );
    }

    #[test]
    fn prune_chunk_counts_full_partial_then_empty_batches() {
        let mut db = Database::open_memory().unwrap();
        let tx = db.conn.transaction().unwrap();
        for _ in 0..2_505 {
            tx.execute(
                "INSERT INTO fixes (at_rfc3339, lat, lon, accuracy_m, n_sources)
                 VALUES ('2010-01-01T00:00:00+00:00', 1.0, 2.0, 3.0, 1)",
                [],
            )
            .unwrap();
        }
        tx.execute(
            "INSERT INTO fixes (at_rfc3339, lat, lon, accuracy_m, n_sources)
             VALUES ('2030-01-01T00:00:00+00:00', 4.0, 5.0, 6.0, 1)",
            [],
        )
        .unwrap();
        tx.commit().unwrap();

        let cutoff = "2020-01-01T00:00:00+00:00";
        assert_eq!(db.prune_fixes_chunk(cutoff, 1_000).unwrap(), 1_000);
        assert_eq!(db.prune_fixes_chunk(cutoff, 1_000).unwrap(), 1_000);
        assert_eq!(db.prune_fixes_chunk(cutoff, 1_000).unwrap(), 505);
        assert_eq!(db.prune_fixes_chunk(cutoff, 1_000).unwrap(), 0);
        assert_eq!(
            db.get_fixes_in_range("2000-01-01T00:00:00+00:00", "2099-01-01T00:00:00+00:00")
                .unwrap()
                .len(),
            1,
            "non-expired fix must remain"
        );
    }

    #[test]
    fn aps_prune_chunk_selection_is_index_search_not_full_scan() {
        let db = Database::open_memory().unwrap();
        let explain = format!("EXPLAIN QUERY PLAN {PRUNE_APS_CHUNK_SQL}");
        let mut stmt = db.conn.prepare(&explain).unwrap();
        let details: Vec<String> = stmt
            .query_map(params!["2020-01-01T00:00:00+00:00", 1_000_i64], |row| {
                row.get(3)
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            details.iter().any(|detail| {
                detail.contains("SEARCH aps") && detail.contains("idx_aps_last_seen")
            }),
            "expired AP selection must SEARCH idx_aps_last_seen: {details:?}"
        );
        assert!(
            details.iter().all(|detail| !detail.contains("SCAN aps")),
            "AP prune plan must never full-scan aps: {details:?}"
        );
    }

    #[test]
    fn aps_prune_strict_cutoff_and_chunk_counts() {
        let mut db = Database::open_memory().unwrap();
        let tx = db.conn.transaction().unwrap();
        for index in 0..2_505 {
            tx.execute(
                "INSERT INTO aps (
                    bssid, lat, lon, source, source_priority,
                    first_seen, last_seen, fetched_at
                 ) VALUES (?1, 1.0, 2.0, 'apple', 40, ?2, ?2, ?2)",
                params![format!("old-{index:04}"), "2010-01-01T00:00:00+00:00"],
            )
            .unwrap();
        }
        for (bssid, seen) in [
            ("at-cutoff", "2020-01-01T00:00:00+00:00"),
            ("new", "2030-01-01T00:00:00+00:00"),
        ] {
            tx.execute(
                "INSERT INTO aps (
                    bssid, lat, lon, source, source_priority,
                    first_seen, last_seen, fetched_at
                 ) VALUES (?1, 1.0, 2.0, 'apple', 40, ?2, ?2, ?2)",
                params![bssid, seen],
            )
            .unwrap();
        }
        tx.commit().unwrap();

        let cutoff = "2020-01-01T00:00:00+00:00";
        assert_eq!(db.prune_aps_chunk(cutoff, 1_000).unwrap(), 1_000);
        assert_eq!(db.prune_aps_chunk(cutoff, 1_000).unwrap(), 1_000);
        assert_eq!(db.prune_aps_chunk(cutoff, 1_000).unwrap(), 505);
        assert_eq!(db.prune_aps_chunk(cutoff, 1_000).unwrap(), 0);
        let remaining: Vec<String> = db
            .conn
            .prepare("SELECT bssid FROM aps ORDER BY bssid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(remaining, vec!["at-cutoff", "new"]);
    }

    #[test]
    fn retention_cutoff_zero_negative_and_huge_are_safe() {
        assert_eq!(retention_cutoff(0).unwrap(), None);
        assert!(retention_cutoff(-1).is_err());
        assert!(retention_cutoff(MAX_RETENTION_DAYS + 1).is_err());

        let db = Database::open_memory().unwrap();
        let ap = sample_ap(1);
        db.upsert_ap(&ap).unwrap();
        db.conn
            .execute("UPDATE aps SET last_seen = '1000-01-01T00:00:00+00:00'", [])
            .unwrap();
        assert_eq!(db.prune_aps(0).unwrap(), 0);
        assert!(db.prune_aps(-1).is_err());
        assert!(db.prune_aps(MAX_RETENTION_DAYS + 1).is_err());
        assert_eq!(db.cached_ap_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn actor_fifo_touch_refreshes_ap_before_retention_prune() {
        let (runtime, handle, legacy) = DbRuntime::open_test().unwrap();
        let ap = sample_ap(2);
        legacy.upsert_ap(&ap).unwrap();
        legacy
            .conn
            .execute(
                "UPDATE aps SET last_seen = '2000-01-01T00:00:00+00:00' WHERE bssid = ?1",
                params![&ap.bssid],
            )
            .unwrap();

        assert_eq!(
            handle
                .lookup_cached_touch(vec![ap.bssid.clone()])
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(handle.prune_aps(30).await.unwrap(), 0);
        assert_eq!(handle.stats().await.unwrap().cached_aps, 1);
        drop(handle);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn configured_actor_denylist_reaches_every_cache_upsert() {
        let dir = unique_tempdir("task0093-configured-denylist");
        let path = dir.join("whereami.db");
        let (runtime, handle) =
            DbRuntime::open_with_mobile_ssid_denylist(&path, vec!["Shuttle-*".to_owned()])
                .await
                .unwrap();
        let mut ap = sample_ap(3);
        ap.ssid = Some("shuttle-42".to_owned());

        handle.cache_resolved_ap(ap.clone(), false).await.unwrap();

        let stored = handle
            .lookup_cached_touch(vec![ap.bssid])
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert!(stored.is_mobile);
        drop(handle);
        runtime.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    fn seed_every_privacy_table(db: &Database) {
        db.upsert_ap(&sample_ap(1)).unwrap();
        db.insert_pending("pending", Some("ssid"), Some(2437), Some(-40))
            .unwrap();
        db.insert_not_found("not-found").unwrap();
        db.insert_fix("2026-01-01T00:00:00+00:00", 1.0, 2.0, 3.0, 4)
            .unwrap();
        db.set_last_fix(&LastFix {
            lat: 1.0,
            lon: 2.0,
            accuracy_m: 3.0,
            address: Some("private address".to_string()),
            at: ts("2026-01-01T00:00:00+00:00"),
            sources: 4,
        })
        .unwrap();
    }

    #[test]
    fn privacy_wipe_is_atomic_counted_idempotent_and_reusable() {
        let mut db = Database::open_memory().unwrap();
        seed_every_privacy_table(&db);
        db.set_metadata("wigle_lookups_today", "7").unwrap();
        db.set_metadata("api_calls_date", "2026-07-05").unwrap();
        let schema_before = db.get_schema_version();
        let sequence_before: i64 = db
            .conn
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name = 'fixes'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sequence_before, 1);

        let counts = db.wipe_privacy_data().unwrap();
        assert_eq!(
            counts,
            PrivacyWipeCounts {
                aps_deleted: 1,
                pending_deleted: 1,
                not_found_deleted: 1,
                fixes_deleted: 1,
                last_fix_deleted: 1,
            }
        );
        assert_eq!(db.cached_ap_count().unwrap(), 0);
        assert_eq!(db.pending_ap_count().unwrap(), 0);
        assert_eq!(db.not_found_ap_count().unwrap(), 0);
        assert!(db.get_fixes_in_range("0000", "9999").unwrap().is_empty());
        assert!(db.get_last_fix().unwrap().is_none());
        assert_eq!(db.get_schema_version(), schema_before);
        assert_eq!(
            db.get_metadata("wigle_lookups_today").unwrap().as_deref(),
            Some("7")
        );
        assert_eq!(
            db.get_metadata("api_calls_date").unwrap().as_deref(),
            Some("2026-07-05")
        );
        let sequence_after: Option<i64> = db
            .conn
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name = 'fixes'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(sequence_after, None);
        assert_eq!(
            db.wipe_privacy_data().unwrap(),
            PrivacyWipeCounts::default()
        );

        // Normal operation resumes, and resetting sqlite_sequence makes the
        // first new history row start from a clean high-water mark.
        seed_every_privacy_table(&db);
        let new_id: i64 = db
            .conn
            .query_row("SELECT id FROM fixes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(new_id, 1);
    }

    #[test]
    fn privacy_wipe_rolls_back_every_table_on_mid_transaction_error() {
        let mut db = Database::open_memory().unwrap();
        seed_every_privacy_table(&db);
        db.conn
            .execute_batch(
                "CREATE TRIGGER reject_fix_wipe BEFORE DELETE ON fixes
                 BEGIN SELECT RAISE(ABORT, 'injected wipe failure'); END;",
            )
            .unwrap();

        let error = db.wipe_privacy_data().unwrap_err();
        assert!(error.to_string().contains("injected wipe failure"));
        assert_eq!(db.cached_ap_count().unwrap(), 1);
        assert_eq!(db.pending_ap_count().unwrap(), 1);
        assert_eq!(db.not_found_ap_count().unwrap(), 1);
        assert_eq!(db.get_fixes_in_range("0000", "9999").unwrap().len(), 1);
        assert!(db.get_last_fix().unwrap().is_some());
    }

    #[test]
    fn file_database_uses_wal_normal_and_named_busy_timeout() {
        let dir = unique_tempdir("task0192-pragmas");
        let path = dir.join("whereami.db");
        let db = Database::open(&path).unwrap();
        let journal: String = db
            .conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        let synchronous: i64 = db
            .conn
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .unwrap();
        let busy_timeout_ms: u64 = db
            .conn
            .pragma_query_value(None, "busy_timeout", |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_ascii_lowercase(), "wal");
        assert_eq!(synchronous, 1, "SQLite NORMAL is numeric mode 1");
        assert_eq!(busy_timeout_ms, DB_BUSY_TIMEOUT.as_millis() as u64);
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reader_open_never_creates_a_missing_database() {
        let dir = unique_tempdir("task0192-reader-no-create");
        let path = dir.join("missing.db");
        assert!(Database::open_reader(&path).is_err());
        assert!(!path.exists(), "reader flags must omit SQLITE_OPEN_CREATE");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reader_startup_failure_stops_writer_before_returning() {
        let dir = unique_tempdir("task0192-reader-startup-cleanup");
        let path = dir.join("whereami.db");
        let failed_path = path.clone();
        let result = tokio::task::spawn_blocking(move || {
            DbRuntime::open_blocking_with_reader(
                failed_path,
                |writer_path| Database::open(&writer_path),
                |_| anyhow::bail!("injected reader startup failure"),
            )
        })
        .await
        .unwrap();
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("injected reader failure must fail startup"),
        };
        assert!(format!("{error:#}").contains("injected reader startup failure"));

        // The failure helper returns only after stop_blocking joined the
        // writer. A fresh runtime can own the same migrated DB immediately.
        let (runtime, handle) = DbRuntime::open(&path).await.unwrap();
        assert_eq!(handle.stats().await.unwrap().cached_aps, 0);
        drop(handle);
        runtime.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn actor_drop_stops_and_joins_even_with_surviving_sender() {
        let actor = DbActor::spawn("whereamid-test-local-drop", Database::open_memory).unwrap();
        let sender = actor.tx.clone();
        let (job_started_tx, job_started_rx) = mpsc::channel();
        let (release_job_tx, release_job_rx) = mpsc::channel();
        sender
            .send(DbCommand::Run(Box::new(move |_| {
                job_started_tx.send(()).unwrap();
                release_job_rx.recv().unwrap();
            })))
            .unwrap();
        job_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("actor job must start");

        let (drop_started_tx, drop_started_rx) = mpsc::channel();
        let (cleanup_complete_tx, cleanup_complete_rx) = mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop_started_tx.send(()).unwrap();
            drop(actor);
            // DbActor::drop returns only after its actor thread is joined.
            cleanup_complete_tx.send(()).unwrap();
        });
        drop_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("actor drop must start");
        release_job_tx.send(()).unwrap();
        cleanup_complete_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("actor Drop did not complete cleanup");
        dropper.join().unwrap();

        assert!(
            sender.send(DbCommand::Run(Box::new(|_| {}))).is_err(),
            "surviving sender must observe the joined actor receiver as closed"
        );
    }

    #[test]
    fn ordinary_actor_job_panic_is_recoverable() {
        let actor =
            DbActor::spawn("whereamid-test-recoverable-panic", Database::open_memory).unwrap();
        let sender = actor.tx.clone();
        let (panicking_started_tx, panicking_started_rx) = mpsc::channel();
        sender
            .send(DbCommand::Run(Box::new(move |_| {
                panicking_started_tx.send(()).unwrap();
                panic!("injected recoverable actor-job panic");
            })))
            .unwrap();
        panicking_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("ordinary panicking job must start");

        let (followup_tx, followup_rx) = mpsc::channel();
        sender
            .send(DbCommand::Run(Box::new(move |db| {
                let reusable = db.cached_ap_count().unwrap() == 0;
                followup_tx.send(reusable).unwrap();
            })))
            .unwrap();
        assert!(
            followup_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("actor must execute work after an ordinary job panic"),
            "ordinary panic recovery must leave the database connection usable"
        );

        drop(sender);
        drop(actor);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actor_threads_own_sql_and_shutdown_closes_channels() {
        let dir = unique_tempdir("task0192-actor-lifecycle");
        let path = dir.join("whereami.db");
        let (runtime, handle) = DbRuntime::open(&path).await.unwrap();
        let (writer, reader) = handle.actor_observations().await.unwrap();
        assert_eq!(writer.0, "whereamid-db-writer");
        assert_eq!(reader.0, "whereamid-db-reader");
        for (_, journal, synchronous, busy_timeout_ms) in [writer, reader] {
            assert_eq!(journal.to_ascii_lowercase(), "wal");
            assert_eq!(synchronous, 1, "SQLite NORMAL is numeric mode 1");
            assert_eq!(busy_timeout_ms, DB_BUSY_TIMEOUT.as_millis() as u64);
        }
        runtime.shutdown().await.unwrap();
        assert!(
            handle.stats().await.is_err(),
            "commands after lifecycle shutdown must fail, not hang"
        );
        drop(handle);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelling_explicit_shutdown_does_not_detach_actor_cleanup() {
        let dir = unique_tempdir("task0192-cancelled-shutdown");
        let path = dir.join("whereami.db");
        let (runtime, handle) = DbRuntime::open(&path).await.unwrap();

        let (writer_blocked_tx, writer_blocked_rx) = mpsc::channel();
        let (release_writer_tx, release_writer_rx) = mpsc::channel();
        let blocker_handle = handle.clone();
        let blocker = tokio::spawn(async move {
            blocker_handle
                .hold_writer_until(writer_blocked_tx, release_writer_rx)
                .await
        });
        writer_blocked_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("writer blocker must start");

        let (writer_stop_queued_tx, writer_stop_queued_rx) = tokio::sync::oneshot::channel();
        let (cleanup_complete_tx, cleanup_complete_rx) = tokio::sync::oneshot::channel();
        let shutdown =
            tokio::spawn(runtime.shutdown_with_signals(writer_stop_queued_tx, cleanup_complete_tx));

        writer_stop_queued_rx
            .await
            .expect("blocking cleanup must queue writer Stop");
        shutdown.abort();
        assert!(
            shutdown.await.unwrap_err().is_cancelled(),
            "outer shutdown waiter must be cancelled for this regression"
        );

        release_writer_tx.send(()).unwrap();
        blocker.await.unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(1), cleanup_complete_rx)
            .await
            .expect("detached cleanup did not finish both actor joins")
            .expect("cleanup completion signal dropped before both actor joins");

        assert!(
            handle.writer_barrier().await.is_err(),
            "writer receiver survived cancelled explicit shutdown"
        );
        assert!(
            handle.stats().await.is_err(),
            "reader receiver survived cancelled explicit shutdown"
        );
        drop(handle);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_drop_stops_and_joins_actors_even_with_surviving_handle() {
        let dir = unique_tempdir("task0192-runtime-drop");
        let path = dir.join("whereami.db");
        let (runtime, handle) = DbRuntime::open(&path).await.unwrap();
        let survivor = handle.clone();
        drop(handle);
        drop(runtime);

        assert!(
            tokio::time::timeout(Duration::from_secs(1), survivor.stats())
                .await
                .expect("surviving handle call hung after runtime Drop")
                .is_err(),
            "runtime Drop must close actor receivers despite surviving senders"
        );
        drop(survivor);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wal_reader_and_tokio_timer_respond_while_writer_transaction_is_blocked() {
        let dir = unique_tempdir("task0192-reader-responsive");
        let path = dir.join("whereami.db");
        let (runtime, handle) = DbRuntime::open(&path).await.unwrap();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let blocker_handle = handle.clone();
        let blocker = tokio::spawn(async move {
            blocker_handle
                .hold_writer_transaction_until(entered_tx, release_rx)
                .await
        });
        entered_rx.await.unwrap();

        let stats = tokio::time::timeout(Duration::from_millis(250), handle.stats()).await;
        let timer = tokio::time::timeout(
            Duration::from_millis(250),
            tokio::time::sleep(Duration::from_millis(1)),
        )
        .await;

        release_tx.send(()).unwrap();
        blocker.await.unwrap().unwrap();
        assert!(timer.is_ok(), "blocked SQLite writer stalled Tokio timer");
        assert!(
            matches!(stats, Ok(Ok(_))),
            "WAL reader did not serve stats while writer was blocked: {stats:?}"
        );
        drop(handle);
        runtime.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn external_writer_hits_named_busy_timeout_then_retry_succeeds() {
        let dir = unique_tempdir("task0192-busy-timeout");
        let path = dir.join("whereami.db");
        let (runtime, handle) = DbRuntime::open(&path).await.unwrap();
        let external = Connection::open(&path).unwrap();
        external.execute_batch("BEGIN IMMEDIATE").unwrap();

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let write_handle = handle.clone();
        let write = tokio::spawn(async move {
            write_handle
                .set_metadata_started("busy-test".to_owned(), "ok".to_owned(), started_tx)
                .await
        });
        started_rx.await.unwrap();
        let started = std::time::Instant::now();
        let error = tokio::time::timeout(DB_BUSY_TIMEOUT + Duration::from_secs(1), write)
            .await
            .expect("writer exceeded the named busy timeout watchdog")
            .unwrap()
            .expect_err("external BEGIN IMMEDIATE must hold the writer through timeout");
        let elapsed = started.elapsed();
        assert!(
            elapsed >= DB_BUSY_TIMEOUT.saturating_sub(Duration::from_millis(250)),
            "SQLite returned before the configured busy timeout: {elapsed:?}"
        );
        let sqlite = error
            .downcast_ref::<rusqlite::Error>()
            .expect("busy failure must preserve rusqlite root cause");
        let code = match sqlite {
            rusqlite::Error::SqliteFailure(error, _) => error.code,
            other => panic!("expected SQLite busy/locked failure, got {other:?}"),
        };
        assert!(matches!(
            code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        ));

        external.execute_batch("ROLLBACK").unwrap();
        handle.set_metadata("busy-test", "ok").await.unwrap();
        assert_eq!(
            handle.get_metadata("busy-test").await.unwrap().as_deref(),
            Some("ok")
        );

        drop(handle);
        runtime.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reservation_begins_in_write_state() {
        let mut db = Database::open_memory().unwrap();
        let mut observed = None;
        let reserved = db
            .try_reserve_wigle_lookup_observing(1, |state| observed = Some(state))
            .unwrap();
        assert_eq!(reserved, QuotaDecision::Reserved);
        assert_eq!(observed, Some(rusqlite::TransactionState::Write));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_queued_write_is_skipped_before_stop_or_later_work() {
        let (runtime, handle, _legacy) = DbRuntime::open_test().unwrap();
        let (blocked_tx, blocked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let blocker_handle = handle.clone();
        let blocker = tokio::spawn(async move {
            blocker_handle
                .hold_writer_until(blocked_tx, release_rx)
                .await
        });
        blocked_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("writer blocker must start");

        let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
        let queued_handle = handle.clone();
        let queued = tokio::spawn(async move {
            queued_handle
                .set_metadata_queued("cancelled-write".to_owned(), "bad".to_owned(), queued_tx)
                .await
        });
        queued_rx.await.unwrap();
        queued.abort();
        assert!(queued.await.unwrap_err().is_cancelled());
        release_tx.send(()).unwrap();
        blocker.await.unwrap().unwrap();
        handle.writer_barrier().await.unwrap();

        assert_eq!(handle.get_metadata("cancelled-write").await.unwrap(), None);
        drop(handle);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_queued_privacy_wipe_still_commits_durably() {
        let (runtime, handle, legacy) = DbRuntime::open_test().unwrap();
        seed_every_privacy_table(&legacy);
        let (blocked_tx, blocked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let blocker_handle = handle.clone();
        let blocker = tokio::spawn(async move {
            blocker_handle
                .hold_writer_until(blocked_tx, release_rx)
                .await
        });
        blocked_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("writer blocker must start");

        let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
        let wipe_handle = handle.clone();
        let wipe =
            tokio::spawn(async move { wipe_handle.wipe_privacy_data_queued(queued_tx).await });
        queued_rx.await.unwrap();
        wipe.abort();
        assert!(wipe.await.unwrap_err().is_cancelled());
        release_tx.send(()).unwrap();
        blocker.await.unwrap().unwrap();
        handle.writer_barrier().await.unwrap();

        let stats = handle.stats().await.unwrap();
        assert_eq!(stats.cached_aps, 0);
        assert_eq!(stats.pending_aps, 0);
        assert_eq!(stats.not_found_aps, 0);
        assert!(handle.get_last_fix().await.unwrap().is_none());
        assert!(handle
            .get_fixes_in_range("0000", "9999")
            .await
            .unwrap()
            .is_empty());
        drop(handle);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_queued_reader_is_skipped_and_reader_is_reusable() {
        let (runtime, handle, _legacy) = DbRuntime::open_test().unwrap();
        let (blocked_tx, blocked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let blocker_handle = handle.clone();
        let blocker = tokio::spawn(async move {
            blocker_handle
                .hold_reader_until(blocked_tx, release_rx)
                .await
        });
        blocked_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("reader blocker must start");

        let executed = Arc::new(AtomicBool::new(false));
        let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
        let queued_handle = handle.clone();
        let probe_executed = Arc::clone(&executed);
        let queued = tokio::spawn(async move {
            queued_handle
                .queued_reader_probe(probe_executed, queued_tx)
                .await
        });
        queued_rx.await.unwrap();
        queued.abort();
        assert!(queued.await.unwrap_err().is_cancelled());
        release_tx.send(()).unwrap();
        blocker.await.unwrap().unwrap();

        assert!(
            !executed.load(Ordering::SeqCst),
            "cancelled queued reader operation must not start"
        );
        assert_eq!(handle.stats().await.unwrap().cached_aps, 0);
        drop(handle);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queued_guarded_history_cancellation_holds_admission_until_actor_skip() {
        let (runtime, handle, _legacy) = DbRuntime::open_test().unwrap();
        let (blocked_tx, blocked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let blocker_handle = handle.clone();
        let blocker = tokio::spawn(async move {
            blocker_handle
                .hold_reader_until(blocked_tx, release_rx)
                .await
        });
        blocked_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("reader blocker must start");

        let admission = Arc::new(tokio::sync::Semaphore::new(1));
        let lease = Arc::new(
            Arc::clone(&admission)
                .try_acquire_owned()
                .expect("history admission"),
        );
        let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
        let queued_handle = handle.clone();
        let command_lease = Arc::clone(&lease);
        let waiter = tokio::spawn(async move {
            queued_handle
                .get_fixes_in_range_guarded_with_hooks(
                    "2026-07-05T00:00:00+00:00",
                    "2026-07-05T01:00:00+00:00",
                    command_lease,
                    Some(queued_tx),
                    || {},
                )
                .await
        });
        drop(lease);
        queued_rx.await.expect("guarded command must be queued");
        waiter.abort();
        let waiter_result = waiter.await;
        let held_before_release = Arc::clone(&admission).try_acquire_owned().is_err();

        // Cleanup before assertions. Releasing the occupied reader lets it
        // process the cancelled command, whose skipped closure owns the lease.
        release_tx.send(()).unwrap();
        let blocker_result = blocker.await.unwrap();
        let restored = tokio::time::timeout(
            Duration::from_secs(1),
            Arc::clone(&admission).acquire_owned(),
        )
        .await
        .expect("actor did not drop cancelled queued history command")
        .expect("history admission semaphore closed");
        drop(restored);
        let final_capacity = admission.available_permits();
        drop(handle);
        runtime.shutdown().await.unwrap();

        assert!(waiter_result.unwrap_err().is_cancelled());
        blocker_result.unwrap();
        assert!(
            held_before_release,
            "queued actor command must retain history admission after waiter abort"
        );
        assert_eq!(final_capacity, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn active_guarded_history_cancellation_holds_admission_until_query_unwinds() {
        let (runtime, handle, _legacy) = DbRuntime::open_test().unwrap();
        let admission = Arc::new(tokio::sync::Semaphore::new(1));
        let lease = Arc::new(
            Arc::clone(&admission)
                .try_acquire_owned()
                .expect("history admission"),
        );
        let (active_tx, active_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let active_handle = handle.clone();
        let command_lease = Arc::clone(&lease);
        let waiter = tokio::spawn(async move {
            active_handle
                .get_fixes_in_range_guarded_with_hooks(
                    "2026-07-05T00:00:00+00:00",
                    "2026-07-05T01:00:00+00:00",
                    command_lease,
                    None,
                    move || {
                        let _ = active_tx.send(());
                        release_rx.recv().expect("active query release");
                    },
                )
                .await
        });
        drop(lease);
        active_rx
            .await
            .expect("guarded command must become active on reader actor");
        waiter.abort();
        let waiter_result = waiter.await;
        let held_before_release = Arc::clone(&admission).try_acquire_owned().is_err();

        // Cleanup before assertions. The real interruptible command observes
        // waiter cancellation after this per-call gate and unwinds the query.
        release_tx.send(()).unwrap();
        let restored = tokio::time::timeout(
            Duration::from_secs(1),
            Arc::clone(&admission).acquire_owned(),
        )
        .await
        .expect("active guarded query did not unwind and release admission")
        .expect("history admission semaphore closed");
        drop(restored);
        let reusable = tokio::time::timeout(Duration::from_secs(1), handle.stats()).await;
        let final_capacity = admission.available_permits();
        drop(handle);
        runtime.shutdown().await.unwrap();

        assert!(waiter_result.unwrap_err().is_cancelled());
        assert!(
            held_before_release,
            "active reader command must retain history admission after waiter abort"
        );
        reusable
            .expect("reader actor was not reusable after guarded cancellation")
            .expect("reader stats failed after guarded cancellation");
        assert_eq!(final_capacity, 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelling_active_long_read_interrupts_sql_and_reader_is_reusable() {
        let dir = unique_tempdir("task0192-interrupt-read");
        let path = dir.join("whereami.db");
        let (runtime, handle) = DbRuntime::open(&path).await.unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let read_handle = handle.clone();
        let read = tokio::spawn(async move { read_handle.run_long_read(started_tx).await });
        started_rx.await.unwrap();
        read.abort();
        assert!(read.await.unwrap_err().is_cancelled());

        let stats = tokio::time::timeout(Duration::from_secs(1), handle.stats())
            .await
            .expect("cancelled long read did not release reader actor")
            .expect("reader actor was not reusable after interruption");
        assert_eq!(stats.cached_aps, 0);
        drop(handle);
        runtime.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn quota_reservation_cancelled_while_queued_cannot_leak() {
        let (runtime, handle, _legacy) = DbRuntime::open_test().unwrap();
        let failures = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let corruption_denials = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (blocked_tx, blocked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let blocker_handle = handle.clone();
        let blocker = tokio::spawn(async move {
            blocker_handle
                .hold_writer_until(blocked_tx, release_rx)
                .await
        });
        blocked_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("writer blocker must start");

        let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
        let reserve_handle = handle.clone();
        let reserve_failures = Arc::clone(&failures);
        let reserve_corruption_denials = Arc::clone(&corruption_denials);
        let reservation = tokio::spawn(async move {
            reserve_handle
                .reserve_wigle_lookup_with_hooks(
                    1,
                    reserve_failures,
                    reserve_corruption_denials,
                    ReservationTestHooks {
                        queued: Some(queued_tx),
                        ..Default::default()
                    },
                )
                .await
        });
        queued_rx.await.unwrap();
        reservation.abort();
        assert!(matches!(reservation.await, Err(error) if error.is_cancelled()));
        release_tx.send(()).unwrap();
        blocker.await.unwrap().unwrap();
        handle.writer_barrier().await.unwrap();

        assert_eq!(handle.wigle_lookups_today().await.unwrap(), 0);
        assert_eq!(failures.load(Ordering::Relaxed), 0);
        assert_eq!(corruption_denials.load(Ordering::Relaxed), 0);
        drop(handle);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn quota_reservation_cancelled_after_commit_is_refunded() {
        let (runtime, handle, _legacy) = DbRuntime::open_test().unwrap();
        let failures = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let corruption_denials = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let commits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        handle
            .install_writer_commit_counter(Arc::clone(&commits))
            .await
            .unwrap();
        commits.store(0, Ordering::SeqCst);
        let (committed_tx, committed_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let reserve_handle = handle.clone();
        let reserve_failures = Arc::clone(&failures);
        let reserve_corruption_denials = Arc::clone(&corruption_denials);
        let reservation = tokio::spawn(async move {
            reserve_handle
                .reserve_wigle_lookup_with_hooks(
                    1,
                    reserve_failures,
                    reserve_corruption_denials,
                    ReservationTestHooks {
                        committed: Some(committed_tx),
                        release_after_commit: Some(release_rx),
                        ..Default::default()
                    },
                )
                .await
        });
        committed_rx.await.unwrap();
        reservation.abort();
        assert!(matches!(reservation.await, Err(error) if error.is_cancelled()));
        release_tx.send(()).unwrap();
        handle.writer_barrier().await.unwrap();

        assert_eq!(handle.wigle_lookups_today().await.unwrap(), 0);
        assert_eq!(
            commits.load(Ordering::SeqCst),
            2,
            "one reservation COMMIT plus exactly one refund COMMIT expected"
        );
        assert_eq!(failures.load(Ordering::Relaxed), 0);
        assert_eq!(corruption_denials.load(Ordering::Relaxed), 0);
        drop(handle);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn corrupt_reservation_cancelled_while_queued_counts_zero() {
        let (runtime, handle, legacy) = DbRuntime::open_test().unwrap();
        seed_api_counter_for_today(&legacy, "garbage");
        let failures = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let corruption_denials = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (blocked_tx, blocked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let blocker_handle = handle.clone();
        let blocker = tokio::spawn(async move {
            blocker_handle
                .hold_writer_until(blocked_tx, release_rx)
                .await
        });
        blocked_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("writer blocker must start");

        let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
        let reserve_handle = handle.clone();
        let reserve_failures = Arc::clone(&failures);
        let reserve_corruption_denials = Arc::clone(&corruption_denials);
        let reservation = tokio::spawn(async move {
            reserve_handle
                .reserve_wigle_lookup_with_hooks(
                    100,
                    reserve_failures,
                    reserve_corruption_denials,
                    ReservationTestHooks {
                        queued: Some(queued_tx),
                        ..Default::default()
                    },
                )
                .await
        });
        queued_rx.await.unwrap();
        reservation.abort();
        assert!(matches!(reservation.await, Err(error) if error.is_cancelled()));
        release_tx.send(()).unwrap();
        blocker.await.unwrap().unwrap();
        handle.writer_barrier().await.unwrap();

        assert_eq!(corruption_denials.load(Ordering::Relaxed), 0);
        assert_eq!(failures.load(Ordering::Relaxed), 0);
        assert_eq!(
            handle
                .get_metadata("wigle_lookups_today")
                .await
                .unwrap()
                .as_deref(),
            Some("garbage")
        );
        drop(handle);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn corrupt_reservation_waiter_cancelled_after_actor_decision_counts_once() {
        let (runtime, handle, legacy) = DbRuntime::open_test().unwrap();
        seed_api_counter_for_today(&legacy, "garbage");
        let failures = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let corruption_denials = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (decision_tx, decision_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let reserve_handle = handle.clone();
        let reserve_failures = Arc::clone(&failures);
        let reserve_corruption_denials = Arc::clone(&corruption_denials);
        let reservation = tokio::spawn(async move {
            reserve_handle
                .reserve_wigle_lookup_with_hooks(
                    100,
                    reserve_failures,
                    reserve_corruption_denials,
                    ReservationTestHooks {
                        decision: Some(decision_tx),
                        release_after_decision: Some(release_rx),
                        ..Default::default()
                    },
                )
                .await
        });

        assert_eq!(decision_rx.await.unwrap(), QuotaDecision::CounterCorrupt);
        assert_eq!(
            corruption_denials.load(Ordering::Relaxed),
            1,
            "actor must count before publishing the decision boundary"
        );
        reservation.abort();
        assert!(matches!(reservation.await, Err(error) if error.is_cancelled()));
        release_tx.send(()).unwrap();
        handle.writer_barrier().await.unwrap();

        assert_eq!(corruption_denials.load(Ordering::Relaxed), 1);
        assert_eq!(failures.load(Ordering::Relaxed), 0);
        assert_eq!(
            handle
                .get_metadata("wigle_lookups_today")
                .await
                .unwrap()
                .as_deref(),
            Some("garbage")
        );
        drop(handle);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actor_quota_decisions_count_only_counter_corruption() {
        let (runtime, handle, legacy) = DbRuntime::open_test().unwrap();
        let failures = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let corruption_denials = Arc::new(std::sync::atomic::AtomicU64::new(0));
        seed_api_counter_for_today(&legacy, "1");

        let exhausted = handle
            .reserve_wigle_lookup(1, Arc::clone(&failures), Arc::clone(&corruption_denials))
            .await
            .unwrap();
        assert!(matches!(exhausted, QuotaReservation::Exhausted));
        assert_eq!(corruption_denials.load(Ordering::Relaxed), 0);

        legacy
            .set_metadata("wigle_lookups_today", "garbage")
            .unwrap();
        let corrupt = handle
            .reserve_wigle_lookup(100, Arc::clone(&failures), Arc::clone(&corruption_denials))
            .await
            .unwrap();
        assert!(matches!(corrupt, QuotaReservation::CounterCorrupt));
        assert_eq!(corruption_denials.load(Ordering::Relaxed), 1);
        assert_eq!(failures.load(Ordering::Relaxed), 0);
        assert_eq!(
            handle
                .get_metadata("wigle_lookups_today")
                .await
                .unwrap()
                .as_deref(),
            Some("garbage")
        );

        drop(handle);
        runtime.shutdown().await.unwrap();
    }

    #[test]
    fn two_connections_cannot_overbook_one_quota_slot() {
        let dir = unique_tempdir("task0192-immediate-reservation");
        let path = dir.join("whereami.db");
        let mut first = Database::open(&path).unwrap();
        let mut second = Database::open(&path).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let first_barrier = Arc::clone(&barrier);
        let first_thread = std::thread::spawn(move || {
            first_barrier.wait();
            first.try_reserve_wigle_lookup(1)
        });
        let second_barrier = Arc::clone(&barrier);
        let second_thread = std::thread::spawn(move || {
            second_barrier.wait();
            second.try_reserve_wigle_lookup(1)
        });
        barrier.wait();
        let outcomes = [first_thread.join().unwrap(), second_thread.join().unwrap()];
        assert!(outcomes.iter().all(Result::is_ok));
        assert_eq!(
            outcomes
                .into_iter()
                .filter(|outcome| outcome.as_ref().unwrap() == &QuotaDecision::Reserved)
                .count(),
            1,
            "IMMEDIATE reservation must grant exactly one slot without SQLITE_BUSY"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_schema_creation() {
        let db = Database::open_memory().unwrap();
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);
    }

    /// Helper: how many rows exist in schema_version, regardless of shape.
    fn schema_version_row_count(db: &Database) -> i64 {
        db.conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .unwrap()
    }

    /// migrate() must be idempotent: calling it twice on a fresh in-memory
    /// DB must leave exactly one row in schema_version, with version =
    /// SCHEMA_VERSION. This is the headline invariant from task 0030.
    #[test]
    fn migrate_is_idempotent_and_leaves_single_row() {
        let db = Database::open_memory().unwrap();
        // open_memory() already calls migrate() once. Call it again.
        db.migrate().unwrap();

        assert_eq!(
            schema_version_row_count(&db),
            1,
            "schema_version must contain exactly one row"
        );
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);

        // And a third time, just to be paranoid.
        db.migrate().unwrap();
        assert_eq!(schema_version_row_count(&db), 1);
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);
    }

    /// The new schema_version table has a CHECK (id = 1) constraint.
    /// Attempting to insert a second row must fail.
    #[test]
    fn schema_version_rejects_second_row() {
        let db = Database::open_memory().unwrap();
        let err = db
            .conn
            .execute(
                "INSERT INTO schema_version (id, version) VALUES (2, 99)",
                [],
            )
            .unwrap_err();
        let msg = format!("{err}");
        // Either CHECK constraint or PRIMARY KEY uniqueness on a duplicate
        // value would qualify. id=2 should fail the CHECK.
        assert!(
            msg.contains("CHECK") || msg.contains("constraint") || msg.contains("UNIQUE"),
            "expected constraint violation, got: {msg}"
        );
    }

    /// A malformed v1/v2 database with multiple rows in schema_version must
    /// be coalesced to a single row at MAX(version) by the v2->v3 migration.
    #[test]
    fn migrate_collapses_malformed_multirow_schema_version() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version VALUES (1);
             INSERT INTO schema_version VALUES (2);
             INSERT INTO schema_version VALUES (1);
             CREATE TABLE aps (
                 bssid TEXT PRIMARY KEY, ssid TEXT, lat REAL NOT NULL, lon REAL NOT NULL,
                 encryption TEXT, channel INTEGER, frequency INTEGER, city TEXT, country TEXT,
                 source TEXT NOT NULL,
                 source_priority INTEGER NOT NULL DEFAULT 0,
                 first_seen TEXT NOT NULL, last_seen TEXT NOT NULL, fetched_at TEXT NOT NULL
             );
             CREATE TABLE not_found (bssid TEXT PRIMARY KEY, first_seen TEXT, last_seen TEXT, checked_at TEXT);
             CREATE TABLE pending (bssid TEXT PRIMARY KEY, ssid TEXT, channel INTEGER, frequency INTEGER, signal_dbm INTEGER, first_seen TEXT, last_seen TEXT, attempts INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        let db = Database {
            conn,
            mobile_ssid_denylist: crate::scanner::default_mobile_ssid_denylist(),
        };
        // Looks like v2 to get_schema_version() because LIMIT 1 returns one
        // of the rows; we don't care which. The point is migrate() must
        // collapse to a single row regardless.
        db.migrate().unwrap();

        assert_eq!(schema_version_row_count(&db), 1);
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);

        // Constraint must now be active.
        let err = db
            .conn
            .execute(
                "INSERT INTO schema_version (id, version) VALUES (2, 99)",
                [],
            )
            .unwrap_err();
        assert!(
            format!("{err}").contains("CHECK")
                || format!("{err}").contains("constraint")
                || format!("{err}").contains("UNIQUE"),
            "expected constraint violation, got: {err}"
        );
    }

    /// Helper: is the schema_version table the v3 single-row shape?
    fn schema_version_is_single_row_constrained(db: &Database) -> bool {
        db.conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='schema_version'",
                [],
                |r| r.get::<_, String>(0),
            )
            .map(|s| s.contains("CHECK (id = 1)"))
            .unwrap_or(false)
    }

    /// Build a genuine v2 database in memory: create_schema_v1 then
    /// migrate_v1_to_v2. Private fns, reachable from this in-module test.
    fn fresh_v2_db() -> Database {
        let db = Database {
            conn: Connection::open_in_memory().unwrap(),
            mobile_ssid_denylist: crate::scanner::default_mobile_ssid_denylist(),
        };
        db.create_schema_v1().unwrap();
        db.migrate_v1_to_v2().unwrap();
        assert_eq!(db.get_schema_version(), 2, "fixture must start at v2");
        db
    }

    /// task-0111: the v1->v2 backfill hardcodes source priorities in frozen
    /// SQL (apple=40, wigle=30, beacondb=20, manual=10, else=0) and must NOT
    /// call Source::priority() — a migration is frozen archaeology that must
    /// not silently rewrite historical data if the enum is later renumbered.
    /// That discipline is correct, but nothing otherwise asserts the hardcoded
    /// numbers EQUAL Source::priority() AT THIS COMMIT. This test is the
    /// tripwire: if someone renumbers the enum (e.g. Apple 40->50) and forgets
    /// to write a NEW migration, the read-back priority for that source no
    /// longer matches Source::priority() and this assertion fails, saying
    /// "you changed the ladder; write a new migration, don't edit the frozen
    /// one." The unrecognised legacy string exercises the migration's
    /// `NOT IN (...)` -> 0 branch (Source::Unknown -> priority 0); it must
    /// match too.
    #[test]
    fn migrate_v1_to_v2_priorities_match_source_enum() {
        let db = Database {
            conn: Connection::open_in_memory().unwrap(),
            mobile_ssid_denylist: crate::scanner::default_mobile_ssid_denylist(),
        };
        db.create_schema_v1().unwrap();

        // One aps row per source string, using distinct locally-administered
        // (02:00:...) BSSIDs. "legacy_prototype" is unrecognised and exercises
        // the migration's NOT IN (...) -> 0 branch. v1 aps has NOT NULL
        // bssid/lat/lon/source/first_seen/last_seen/fetched_at and NO
        // source_priority column (that is what migrate_v1_to_v2 adds).
        let cases = [
            ("02:00:00:00:00:01", "apple"),
            ("02:00:00:00:00:02", "wigle"),
            ("02:00:00:00:00:03", "beacondb"),
            ("02:00:00:00:00:04", "manual"),
            ("02:00:00:00:00:05", "legacy_prototype"),
        ];
        for (bssid, source) in cases {
            db.conn
                .execute(
                    "INSERT INTO aps
                        (bssid, lat, lon, source, first_seen, last_seen, fetched_at)
                     VALUES (?1, 0.0, 0.0, ?2, 't', 't', 't')",
                    params![bssid, source],
                )
                .unwrap();
        }

        db.migrate_v1_to_v2().unwrap();

        // Read back each row's backfilled source_priority and assert it equals
        // the live enum's priority for that source. This makes the migration's
        // hardcoded numbers and Source::priority() provably equal AT THIS
        // COMMIT.
        for (bssid, source) in cases {
            let stored: i32 = db
                .conn
                .query_row(
                    "SELECT source_priority FROM aps WHERE bssid = ?1",
                    params![bssid],
                    |r| r.get(0),
                )
                .unwrap();
            let expected = Source::from_db_str(source).priority();
            assert_eq!(
                stored, expected,
                "migration backfill for source {source:?} ({stored}) must equal \
                 Source::priority() ({expected}); if these diverge, renumber via \
                 a NEW migration, do not edit migrate_v1_to_v2"
            );
        }
    }

    fn fresh_v6_db() -> Database {
        let db = fresh_v2_db();
        db.migrate_v2_to_v3().unwrap();
        db.migrate_v3_to_v4().unwrap();
        db.migrate_v4_to_v5().unwrap();
        db.migrate_v5_to_v6().unwrap();
        assert_eq!(db.get_schema_version(), 6, "fixture must start at v6");
        db
    }

    /// task-0191 AC#1/#2: step through the migration chain one version at a
    /// time, asserting the stored stamp AND the table shape after EACH step.
    /// This is the direct regression guard for the v2->v3
    /// SCHEMA_VERSION-stamping bug: applied in isolation, migrate_v2_to_v3
    /// must leave the stamp at EXACTLY 3 (not the current version), with
    /// neither last_fix nor fixes present yet.
    #[test]
    fn migration_chain_stamps_each_version_in_lockstep() {
        let db = fresh_v2_db();
        // v2 shape: multi-row schema_version, no v4/v5 tables.
        assert!(!db.table_exists("last_fix"));
        assert!(!db.table_exists("fixes"));

        // --- v2 -> v3 (in isolation): stamp is exactly 3, shape is v3 ---
        db.migrate_v2_to_v3().unwrap();
        assert_eq!(
            db.get_schema_version(),
            3,
            "migrate_v2_to_v3 must stamp exactly 3, not SCHEMA_VERSION"
        );
        assert!(
            schema_version_is_single_row_constrained(&db),
            "v3 shape: schema_version must be single-row constrained"
        );
        assert!(
            !db.table_exists("last_fix"),
            "last_fix must NOT exist at v3"
        );
        assert!(!db.table_exists("fixes"), "fixes must NOT exist at v3");

        // --- v3 -> v4: stamp 4, last_fix appears, fixes still absent ---
        db.migrate_v3_to_v4().unwrap();
        assert_eq!(db.get_schema_version(), 4);
        assert!(db.table_exists("last_fix"), "last_fix present at v4");
        assert!(!db.table_exists("fixes"), "fixes still absent at v4");

        // --- v4 -> v5: stamp 5, fixes appears ---
        db.migrate_v4_to_v5().unwrap();
        assert_eq!(db.get_schema_version(), 5);
        assert!(db.table_exists("last_fix"));
        assert!(db.table_exists("fixes"), "fixes present at v5");
        assert!(!db.column_exists("aps", "is_mobile"));

        // --- v5 -> v6: stamp 6, mobile classification column appears ---
        db.migrate_v5_to_v6().unwrap();
        assert_eq!(db.get_schema_version(), 6);
        assert!(db.column_exists("aps", "is_mobile"));

        // --- v6 -> v7: stamp 7, complete nullable quality bundle appears ---
        db.migrate_v6_to_v7().unwrap();
        assert_eq!(db.get_schema_version(), 7);
        assert!(db.fix_quality_columns_have_expected_shape());

        // --- v7 -> v8: stamp 8, WiGLE counter metadata key renamed ---
        // Seed the legacy counter first so the rename is observable, not a
        // vacuous no-op: the value must survive under the new key.
        db.set_metadata("api_calls_date", "2026-07-09").unwrap();
        db.set_metadata("api_calls_today", "17").unwrap();
        db.migrate_v7_to_v8().unwrap();
        assert_eq!(db.get_schema_version(), 8);
        assert_eq!(
            db.get_metadata("wigle_lookups_today").unwrap().as_deref(),
            Some("17"),
            "v7->v8 must rename the key preserving the value"
        );
        assert!(
            db.get_metadata("api_calls_today").unwrap().is_none(),
            "v7->v8 must leave no stale api_calls_today row"
        );

        // --- v8 -> v9: stamp 9, redundant channel columns dropped (task-0123) ---
        assert!(
            db.column_exists("aps", "channel"),
            "channel present before v9 (fresh v1 DDL carried it)"
        );
        assert!(db.column_exists("pending", "channel"));
        db.migrate_v8_to_v9().unwrap();
        assert_eq!(db.get_schema_version(), 9);
        assert!(
            !db.column_exists("aps", "channel"),
            "v8->v9 must drop aps.channel"
        );
        assert!(
            !db.column_exists("pending", "channel"),
            "v8->v9 must drop pending.channel"
        );
        assert!(
            db.column_exists("aps", "frequency"),
            "v8->v9 must keep the canonical aps.frequency"
        );
    }

    // --- task-0151: real historical-shape DB files opened via the PUBLIC
    // Database::open cascade -----------------------------------------------
    //
    // The lockstep test above steps the migrate fns directly on ONE in-memory
    // connection. That is deliberately distinct from what open() does on a
    // fresh file. These tests instead: build a tempfile DB stamped at exactly
    // a historical shape v (create_schema_v1 + migrate fns 1..=v), close it,
    // then reopen via `Database::open` — driving the whole v -> SCHEMA_VERSION
    // cascade — and assert both the final shape AND that seeded rows survived.
    //
    // AUTHENTICITY: each historical shape is produced solely by running the
    // real migration fns up to v. Those fns ARE the definition of each
    // version, so the on-disk shape cannot drift from what a genuine vN
    // database looked like (no hand-copied DDL that could accidentally include
    // a later version's column). A migration re-ordering or a broken
    // intermediate step that still passes the fresh-DB path can corrupt a real
    // upgrade; opening genuine historical files is what catches it.

    /// RAII throwaway directory for one file-backed migration test. Reuses the
    /// existing `unique_tempdir` (no `tempfile` dependency — the crate
    /// deliberately avoids it). Drop removes the directory including the
    /// `-wal`/`-shm` sidecars `Database::open` creates, so cleanup happens even
    /// if an assertion panics mid-test.
    struct TempDbDir(std::path::PathBuf);

    impl TempDbDir {
        fn new(tag: &str) -> Self {
            TempDbDir(unique_tempdir(tag))
        }

        fn db_path(&self) -> std::path::PathBuf {
            self.0.join("aps.sqlite")
        }
    }

    impl Drop for TempDbDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Compare two f64s without tripping clippy's `float_cmp` under
    /// `-D warnings`. The seeded literal round-trips through SQLite REAL
    /// (IEEE-754 double) unchanged, so a tiny epsilon is ample.
    fn assert_close(actual: f64, expected: f64, what: &str) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "{what}: expected {expected}, got {actual}"
        );
    }

    /// Does an index named `name` exist? (`table_exists` only covers tables.)
    fn index_exists(db: &Database, name: &str) -> bool {
        db.conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1",
                params![name],
                |_| Ok(()),
            )
            .optional()
            .map(|o| o.is_some())
            .unwrap_or(false)
    }

    // Fixed seed values, shared by the builder (write) and the verifier (read)
    // so we can assert exact preservation across the cascade.
    const SEED_AP_BSSID: &str = "aa:bb:cc:dd:ee:01";
    const SEED_AP_SSID: &str = "seed-net";
    const SEED_AP_LAT: f64 = 55.6761;
    const SEED_AP_LON: f64 = 12.5683;
    const SEED_AP_SOURCE: &str = "wigle";
    const SEED_NOT_FOUND_BSSID: &str = "11:22:33:44:55:66";
    const SEED_PENDING_BSSID: &str = "99:88:77:66:55:44";
    const SEED_PENDING_ATTEMPTS: i64 = 2;
    const SEED_LAST_FIX_LAT: f64 = 55.0;
    const SEED_LAST_FIX_ACCURACY: f64 = 42.0;
    const SEED_FIXES_AT: &str = "2020-05-01T00:00:00Z";
    const SEED_FIXES_LAT: f64 = 56.0;
    const SEED_FIXES_N_SOURCES: i64 = 4;

    /// Seed representative rows into exactly the tables that exist at `version`.
    /// aps/not_found/pending exist from v1; last_fix from v4; fixes from v5.
    /// The aps INSERT names only v1 columns (source_priority is added at v2,
    /// is_mobile at v6, both with defaults), so the same statement is valid at
    /// every version and does not smuggle in a newer column.
    fn seed_sample_rows(db: &Database, version: i32) {
        db.conn
            .execute(
                "INSERT INTO aps
                    (bssid, ssid, lat, lon, encryption, channel, frequency,
                     city, country, source, first_seen, last_seen, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, 'wpa2', 6, 2437, 'Copenhagen', 'DK',
                         ?5, '2020-01-01T00:00:00Z', '2020-01-02T00:00:00Z',
                         '2020-01-02T00:00:00Z')",
                params![
                    SEED_AP_BSSID,
                    SEED_AP_SSID,
                    SEED_AP_LAT,
                    SEED_AP_LON,
                    SEED_AP_SOURCE
                ],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO not_found (bssid, first_seen, last_seen, checked_at)
                 VALUES (?1, '2020-02-01T00:00:00Z', '2020-02-02T00:00:00Z',
                         '2020-02-02T00:00:00Z')",
                params![SEED_NOT_FOUND_BSSID],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO pending
                    (bssid, ssid, channel, frequency, signal_dbm,
                     first_seen, last_seen, attempts)
                 VALUES (?1, 'pending-net', 11, 2462, -70,
                         '2020-03-01T00:00:00Z', '2020-03-02T00:00:00Z', ?2)",
                params![SEED_PENDING_BSSID, SEED_PENDING_ATTEMPTS],
            )
            .unwrap();
        if version >= 4 {
            db.conn
                .execute(
                    "INSERT INTO last_fix
                        (id, lat, lon, accuracy_m, address, at_rfc3339, sources)
                     VALUES (1, ?1, 12.0, ?2, 'Somewhere',
                             '2020-04-01T00:00:00Z', 3)",
                    params![SEED_LAST_FIX_LAT, SEED_LAST_FIX_ACCURACY],
                )
                .unwrap();
        }
        if version >= 5 {
            db.conn
                .execute(
                    "INSERT INTO fixes (at_rfc3339, lat, lon, accuracy_m, n_sources)
                     VALUES (?1, ?2, 13.0, 25.0, ?3)",
                    params![SEED_FIXES_AT, SEED_FIXES_LAT, SEED_FIXES_N_SOURCES],
                )
                .unwrap();
        }
    }

    /// Build a tempfile DB stamped at exactly historical shape `target`
    /// (1..=7) and seed it, then close it. Authenticity is guaranteed by
    /// constructing the shape ONLY from `create_schema_v1` + the real migrate
    /// fns up to `target` — never hand-written DDL.
    fn build_historical_shape(path: &Path, target: i32) {
        assert!(
            (1..=8).contains(&target),
            "historical shapes are v1..=v8; v9 is the current shape"
        );
        let db = Database {
            conn: Connection::open(path).unwrap(),
            mobile_ssid_denylist: crate::scanner::default_mobile_ssid_denylist(),
        };
        db.create_schema_v1().unwrap();
        if target >= 2 {
            db.migrate_v1_to_v2().unwrap();
        }
        if target >= 3 {
            db.migrate_v2_to_v3().unwrap();
        }
        if target >= 4 {
            db.migrate_v3_to_v4().unwrap();
        }
        if target >= 5 {
            db.migrate_v4_to_v5().unwrap();
        }
        if target >= 6 {
            db.migrate_v5_to_v6().unwrap();
        }
        if target >= 7 {
            db.migrate_v6_to_v7().unwrap();
        }
        if target >= 8 {
            db.migrate_v7_to_v8().unwrap();
        }
        assert_eq!(
            db.get_schema_version(),
            target,
            "builder must produce a genuine v{target} stamp before seeding"
        );
        seed_sample_rows(&db, target);
        // `db` (and its Connection) drops here, flushing/closing the file so
        // Database::open below reopens it cleanly.
    }

    /// After `Database::open` has cascaded a historical file to the current
    /// version, assert the complete current shape and that the seeded rows
    /// (those that existed at `origin`) survived intact.
    fn assert_current_shape_and_seed(db: &Database, origin: i32) {
        assert_eq!(
            db.get_schema_version(),
            SCHEMA_VERSION,
            "opening a v{origin} file must cascade to v{SCHEMA_VERSION}"
        );

        for table in [
            "schema_version",
            "aps",
            "not_found",
            "pending",
            "metadata",
            "last_fix",
            "fixes",
        ] {
            assert!(
                db.table_exists(table),
                "table {table} must exist after cascade from v{origin}"
            );
        }
        for index in ["idx_aps_geo", "idx_aps_last_seen", "idx_fixes_at"] {
            assert!(
                index_exists(db, index),
                "index {index} must exist after cascade from v{origin}"
            );
        }

        assert!(
            schema_version_is_single_row_constrained(db),
            "schema_version must be single-row constrained after cascade from v{origin}"
        );
        assert_eq!(
            schema_version_row_count(db),
            1,
            "exactly one schema_version row after cascade from v{origin}"
        );

        // Columns added by v2 and v6 migrations of aps.
        assert!(
            db.column_exists("aps", "source_priority"),
            "aps.source_priority (v2) must exist after cascade from v{origin}"
        );
        assert!(
            db.column_exists("aps", "is_mobile"),
            "aps.is_mobile (v6) must exist after cascade from v{origin}"
        );
        // v9 dropped the redundant band-less channel column from both tables
        // (task-0123); frequency is canonical and must survive.
        assert!(
            !db.column_exists("aps", "channel"),
            "aps.channel (v9 drop) must be gone after cascade from v{origin}"
        );
        assert!(
            !db.column_exists("pending", "channel"),
            "pending.channel (v9 drop) must be gone after cascade from v{origin}"
        );
        assert!(
            db.column_exists("aps", "frequency"),
            "aps.frequency (canonical) must survive after cascade from v{origin}"
        );
        // v7 nullable quality bundle on fixes.
        assert!(
            db.fix_quality_columns_have_expected_shape(),
            "fixes v7 quality columns must exist after cascade from v{origin}"
        );

        // Seeded aps row (present at every origin) survived unchanged.
        let (ssid, lat, lon, source): (String, f64, f64, String) = db
            .conn
            .query_row(
                "SELECT ssid, lat, lon, source FROM aps WHERE bssid = ?1",
                params![SEED_AP_BSSID],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(ssid, SEED_AP_SSID, "aps.ssid preserved (origin v{origin})");
        assert_close(lat, SEED_AP_LAT, "aps.lat");
        assert_close(lon, SEED_AP_LON, "aps.lon");
        assert_eq!(
            source, SEED_AP_SOURCE,
            "aps.source preserved (origin v{origin})"
        );

        // not_found + pending survived.
        let not_found_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM not_found WHERE bssid = ?1",
                params![SEED_NOT_FOUND_BSSID],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            not_found_count, 1,
            "not_found row preserved (origin v{origin})"
        );
        let pending_attempts: i64 = db
            .conn
            .query_row(
                "SELECT attempts FROM pending WHERE bssid = ?1",
                params![SEED_PENDING_BSSID],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            pending_attempts, SEED_PENDING_ATTEMPTS,
            "pending row preserved (origin v{origin})"
        );

        // last_fix only seeded when it existed at origin (v4+).
        if origin >= 4 {
            let (lf_lat, lf_accuracy): (f64, f64) = db
                .conn
                .query_row(
                    "SELECT lat, accuracy_m FROM last_fix WHERE id = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_close(lf_lat, SEED_LAST_FIX_LAT, "last_fix.lat");
            assert_close(lf_accuracy, SEED_LAST_FIX_ACCURACY, "last_fix.accuracy_m");
        }

        // fixes only seeded when it existed at origin (v5+).
        if origin >= 5 {
            let (fx_lat, fx_n): (f64, i64) = db
                .conn
                .query_row(
                    "SELECT lat, n_sources FROM fixes WHERE at_rfc3339 = ?1",
                    params![SEED_FIXES_AT],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_close(fx_lat, SEED_FIXES_LAT, "fixes.lat");
            assert_eq!(
                fx_n, SEED_FIXES_N_SOURCES,
                "fixes.n_sources preserved (origin v{origin})"
            );
            // The v6->v7 migration must leave pre-existing rows' quality
            // columns NULL (legacy unknown), not fabricate values.
            let confidence: Option<String> = db
                .conn
                .query_row(
                    "SELECT confidence FROM fixes WHERE at_rfc3339 = ?1",
                    params![SEED_FIXES_AT],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(
                confidence.is_none(),
                "v7 quality columns must be NULL for pre-v7 rows (origin v{origin})"
            );
        }
    }

    /// AC#1/#2/#4: for each historical stamped shape v in {1,2,3,4,5,6,7}, build
    /// a genuine v-shape tempfile with seed rows, open it through the PUBLIC
    /// `Database::open` cascade, and assert it reaches SCHEMA_VERSION with the
    /// full current shape and every seeded row preserved. Distinct from
    /// `migration_chain_stamps_each_version_in_lockstep`, which steps the
    /// migrate fns directly on one in-memory connection and never exercises
    /// `Database::open` on a real historical file.
    #[test]
    fn open_each_historical_shape_cascades_to_current() {
        for origin in 1..=8 {
            let tmp = TempDbDir::new(&format!("task0151-cascade-v{origin}"));
            let path = tmp.db_path();
            build_historical_shape(&path, origin);
            let db = Database::open(&path).unwrap();
            assert_current_shape_and_seed(&db, origin);
        }
    }

    /// task-0123 (AC#4 + invariant #1): a genuine v8 aps row carrying BOTH a
    /// channel value and a frequency opens cleanly at v9 with the redundant
    /// channel column DROPPED and the canonical frequency intact — proving the
    /// migration is lossless for the data that matters and that `DROP COLUMN`
    /// works on the bundled SQLite (not just a fresh-DB no-op). The seed
    /// (`seed_sample_rows`) writes channel 6 / frequency 2437 into the aps row.
    #[test]
    fn open_v8_shape_drops_channel_column_at_v9() {
        let tmp = TempDbDir::new("task0123-v8-to-v9");
        let path = tmp.db_path();
        build_historical_shape(&path, 8);

        // The v8 file genuinely carries the channel column with a live value.
        {
            let raw = Connection::open(&path).unwrap();
            let channel: Option<i64> = raw
                .query_row(
                    "SELECT channel FROM aps WHERE bssid = ?1",
                    params![SEED_AP_BSSID],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(channel, Some(6), "v8 seed must carry a real channel value");
        }

        let db = Database::open(&path).unwrap();
        assert_eq!(
            db.get_schema_version(),
            SCHEMA_VERSION,
            "opening a v8 file must cascade to v{SCHEMA_VERSION}"
        );
        assert!(
            !db.column_exists("aps", "channel"),
            "v8->v9 must drop the aps.channel column"
        );
        assert!(
            !db.column_exists("pending", "channel"),
            "v8->v9 must drop the pending.channel column"
        );

        // The canonical frequency survived, and the band-qualified Channel is
        // re-derivable from it on read (channel 6 == 2.4 GHz channel 6).
        let ap = db.get_ap(SEED_AP_BSSID).unwrap().unwrap();
        assert_eq!(ap.frequency, Some(2437), "frequency must survive the drop");
        assert_eq!(
            ap.frequency
                .and_then(whereami_protocol::Channel::from_frequency),
            Some(whereami_protocol::Channel::TwoFourGhz(6)),
            "the dropped channel is re-derivable from the surviving frequency"
        );
    }

    /// task-0123 AC#6: a frequency round-trips through the normal aps read path
    /// and yields the correct band-qualified Channel on read. 5955 MHz is 6 GHz
    /// channel 1 — the case that a bare-number column could not have
    /// disambiguated from 2.4 GHz channel 1.
    #[test]
    fn aps_frequency_roundtrip_derives_band_qualified_channel() {
        let db = Database::open_memory().unwrap();
        let mut ap = sample_ap(1);
        ap.frequency = Some(5955);
        db.upsert_ap(&ap).unwrap();

        let stored = db.get_ap(&ap.bssid).unwrap().unwrap();
        assert_eq!(
            stored.frequency,
            Some(5955),
            "frequency round-trips verbatim"
        );
        assert_eq!(
            stored
                .frequency
                .and_then(whereami_protocol::Channel::from_frequency),
            Some(whereami_protocol::Channel::SixGhz(1)),
            "5955 MHz derives 6 GHz channel 1 on read"
        );
    }

    /// task-0130 AC#1: the v7->v8 WiGLE-counter key rename is lossless across a
    /// real `Database::open` cascade. Build a genuine v7 file, seed a live
    /// same-day counter under the OLD key `api_calls_today`, then open (which
    /// runs migrate_v7_to_v8) and assert the count survives verbatim under the
    /// NEW key `wigle_lookups_today`, with no stale old-key row left behind and
    /// the schema stamped at v8. A daemon upgrading mid-day must not lose its
    /// spent-quota count.
    #[test]
    fn open_v7_shape_renames_wigle_counter_key_losslessly() {
        let tmp = TempDbDir::new("task0130-v7-to-v8");
        let path = tmp.db_path();
        build_historical_shape(&path, 7);

        // Seed the legacy same-day counter directly on the closed v7 file via a
        // raw connection (NOT Database::open, which would migrate immediately).
        {
            let raw = Connection::open(&path).unwrap();
            raw.execute(
                "INSERT INTO metadata (key, value) VALUES ('api_calls_date', '2026-07-09')",
                [],
            )
            .unwrap();
            raw.execute(
                "INSERT INTO metadata (key, value) VALUES ('api_calls_today', '23')",
                [],
            )
            .unwrap();
        }

        let db = Database::open(&path).unwrap();
        assert_eq!(
            db.get_schema_version(),
            SCHEMA_VERSION,
            "opening a v7 file must cascade to v{SCHEMA_VERSION}"
        );
        assert_eq!(
            db.wigle_lookups_today().unwrap(),
            23,
            "v7->v8 must preserve the live counter value under the new key"
        );
        assert!(
            db.get_metadata("api_calls_today").unwrap().is_none(),
            "v7->v8 must leave no stale api_calls_today row"
        );
        // The date key is intentionally NOT renamed by this migration.
        assert_eq!(
            db.get_metadata("api_calls_date").unwrap().as_deref(),
            Some("2026-07-09"),
            "the paired date key is left untouched by v7->v8"
        );
    }

    /// task-0130: v7->v8 on a v7 file with NO prior counter is a clean no-op —
    /// the new key is simply absent (counter starts fresh at 0), and the open
    /// still succeeds and stamps v8.
    #[test]
    fn open_v7_shape_without_counter_starts_fresh_at_v8() {
        let tmp = TempDbDir::new("task0130-v7-empty-counter");
        let path = tmp.db_path();
        build_historical_shape(&path, 7);

        let db = Database::open(&path).unwrap();
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);
        assert_eq!(
            db.wigle_lookups_today().unwrap(),
            0,
            "absent counter starts fresh at 0 under the new key"
        );
    }

    /// task-0130 review hardening: prove the v7->v8 rename FAILS LOUD rather than
    /// silently mis-counting the WiGLE quota if a v7 file somehow already carries
    /// BOTH the old `api_calls_today` and the new `wigle_lookups_today` keys (only
    /// reachable by manual tampering or a downgrade->upgrade flip that wrote the
    /// new key under an old binary). The rename UPDATE would collide with the
    /// existing `wigle_lookups_today` row on `metadata.key`'s PRIMARY KEY, so the
    /// migration transaction must error and `Database::open` must refuse to start
    /// — picking one value silently would under/over-count the daily quota.
    #[test]
    fn open_v7_shape_with_both_counter_keys_fails_loud_not_silent() {
        let tmp = TempDbDir::new("task0130-v7-both-keys");
        let path = tmp.db_path();
        build_historical_shape(&path, 7);
        {
            let raw = Connection::open(&path).unwrap();
            raw.execute(
                "INSERT INTO metadata (key, value) VALUES ('api_calls_today', '5')",
                [],
            )
            .unwrap();
            raw.execute(
                "INSERT INTO metadata (key, value) VALUES ('wigle_lookups_today', '9')",
                [],
            )
            .unwrap();
        }

        let opened = Database::open(&path);
        assert!(
            opened.is_err(),
            "v7->v8 must fail loud on a key collision, not silently pick a counter value"
        );
    }

    /// AC#3: the v3 -> v7 path specifically. A v3-shape DB has neither the v4
    /// last_fix table, the v5 fixes table, the v6 aps.is_mobile column, nor the
    /// v7 fixes quality columns. Opening it must run migrate_v3_to_v4 /
    /// v4_to_v5 / v5_to_v6 / v6_to_v7 in sequence, producing all of them while
    /// preserving the aps/not_found/pending rows seeded at v3.
    ///
    /// Why this catches a broken intermediate migration: if, say,
    /// migrate_v5_to_v6 were a no-op, opening this v3 file would still stamp 7
    /// (later migrations run and stamp) but aps.is_mobile would be absent — the
    /// `column_exists("aps", "is_mobile")` assertion fails. The shape repair in
    /// migrate() does NOT mask this: it runs off the STORED stamp (3 here, a
    /// genuine v3 shape), so it declines to repair and the real cascade is what
    /// must build every later shape.
    #[test]
    fn open_v3_shape_runs_all_upper_migrations_and_preserves_rows() {
        let tmp = TempDbDir::new("task0151-v3-to-v7");
        let path = tmp.db_path();
        build_historical_shape(&path, 3);

        // Confirm the on-disk file really is a v3 shape before opening: none of
        // the v4/v5/v6/v7 artifacts may be present yet.
        {
            let probe = Database {
                conn: Connection::open(&path).unwrap(),
                mobile_ssid_denylist: crate::scanner::default_mobile_ssid_denylist(),
            };
            assert_eq!(probe.get_schema_version(), 3, "pre-open file must be v3");
            assert!(
                !probe.table_exists("last_fix"),
                "v3 file must lack last_fix"
            );
            assert!(!probe.table_exists("fixes"), "v3 file must lack fixes");
            assert!(
                !probe.column_exists("aps", "is_mobile"),
                "v3 file must lack aps.is_mobile"
            );
            assert!(
                !probe.fix_quality_columns_have_expected_shape(),
                "v3 file must lack the v7 fixes quality columns"
            );
        }

        // Open through the public cascade and assert every upper migration ran
        // and the seeded rows survived.
        let db = Database::open(&path).unwrap();
        assert_current_shape_and_seed(&db, 3);
    }

    /// task-0191 AC#3 (positive): a DB stamped newer than its shape — the
    /// exact strand the old SCHEMA_VERSION bug produced — is detected and
    /// repaired on migrate(), not silently skipped. We manufacture the strand:
    /// a real v3 DB whose stored version is hand-forced to 5 with no v4/v5
    /// tables. migrate() must reset down, re-run the chain, and rebuild the
    /// missing tables.
    #[test]
    fn migrate_repairs_version_stamped_ahead_of_shape() {
        let db = fresh_v2_db();
        db.migrate_v2_to_v3().unwrap();
        assert_eq!(db.get_schema_version(), 3);
        assert!(!db.table_exists("last_fix"));
        assert!(!db.table_exists("fixes"));

        // Forge the strand: stamp 5 while last_fix/fixes still do not exist.
        db.conn
            .execute("UPDATE schema_version SET version = 5 WHERE id = 1", [])
            .unwrap();
        assert_eq!(db.get_schema_version(), 5);

        // Heal: warn + reset to 3 + re-run the complete migration chain.
        db.migrate().unwrap();
        assert_eq!(
            db.get_schema_version(),
            SCHEMA_VERSION,
            "healed DB ends at the current version"
        );
        assert!(db.table_exists("last_fix"), "repair must create last_fix");
        assert!(db.table_exists("fixes"), "repair must create fixes");
        assert!(db.column_exists("aps", "is_mobile"));
    }

    /// task-0191 AC#3 (negative): an intact current DB passes the shape check
    /// untouched — no spurious repair. Re-running migrate() is a no-op:
    /// version stays current and every shape remains present.
    #[test]
    fn migrate_does_not_repair_intact_current_schema() {
        let db = Database::open_memory().unwrap();
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);
        assert!(db.table_exists("last_fix"));
        assert!(db.table_exists("fixes"));
        assert!(db.column_exists("aps", "is_mobile"));

        db.migrate().unwrap();
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);
        assert!(db.table_exists("last_fix"));
        assert!(db.table_exists("fixes"));
        assert!(db.column_exists("aps", "is_mobile"));
    }

    #[test]
    fn migrate_v5_to_v6_defaults_existing_rows_to_fixed() {
        let db = fresh_v2_db();
        db.migrate_v2_to_v3().unwrap();
        db.migrate_v3_to_v4().unwrap();
        db.migrate_v4_to_v5().unwrap();
        db.conn
            .execute(
                "INSERT INTO aps (
                    bssid, ssid, lat, lon, source, source_priority,
                    first_seen, last_seen, fetched_at
                 ) VALUES ('B8:50:01:00:00:01', 'DSB_Wi-Fi', 55.0, 12.0,
                           'apple', 40, 't', 't', 't')",
                [],
            )
            .unwrap();

        db.migrate_v5_to_v6().unwrap();

        assert_eq!(db.get_schema_version(), 6);
        assert!(db.column_exists("aps", "is_mobile"));
        let is_mobile: i64 = db
            .conn
            .query_row(
                "SELECT is_mobile FROM aps WHERE bssid = 'B8:50:01:00:00:01'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            is_mobile, 0,
            "migration must not retroactively classify existing rows"
        );
    }

    #[test]
    fn migrate_v5_to_v6_rolls_back_column_when_stamp_fails() {
        let db = fresh_v2_db();
        db.migrate_v2_to_v3().unwrap();
        db.migrate_v3_to_v4().unwrap();
        db.migrate_v4_to_v5().unwrap();
        db.conn
            .execute_batch(
                "CREATE TRIGGER reject_v6_stamp
                 BEFORE UPDATE ON schema_version
                 WHEN NEW.version = 6
                 BEGIN
                     SELECT RAISE(ABORT, 'injected v6 stamp failure');
                 END;",
            )
            .unwrap();

        let error = db.migrate_v5_to_v6().unwrap_err();
        assert!(format!("{error:#}").contains("injected v6 stamp failure"));
        assert_eq!(db.get_schema_version(), 5);
        assert!(
            !db.column_exists("aps", "is_mobile"),
            "ALTER TABLE must roll back with the failed version stamp"
        );
    }

    #[test]
    fn task0245_migrate_v6_to_v7_preserves_legacy_row_and_exact_column_shape() {
        let db = fresh_v6_db();
        db.conn
            .execute(
                "INSERT INTO fixes (at_rfc3339, lat, lon, accuracy_m, n_sources)
                 VALUES ('2026-07-05T12:00:00+00:00', 55.0, 12.0, 3.0, 4)",
                [],
            )
            .unwrap();

        db.migrate_v6_to_v7().unwrap();

        assert_eq!(db.get_schema_version(), 7);
        let mut statement = db.conn.prepare("PRAGMA table_info(fixes)").unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .unwrap();
        let columns: std::collections::HashMap<_, _> = rows
            .map(|row| {
                let (name, declared_type, not_null, default_value) = row.unwrap();
                (name, (declared_type, not_null, default_value))
            })
            .collect();
        for (name, declared_type) in [
            ("implied_speed_kmh", "REAL"),
            ("last_fix_age_secs", "INTEGER"),
            ("inlier_ratio", "REAL"),
            ("confidence", "TEXT"),
            ("suspect_reason", "TEXT"),
        ] {
            assert_eq!(
                columns.get(name),
                Some(&(declared_type.to_string(), 0, None)),
                "unexpected declaration for fixes.{name}"
            );
        }
        let quality_nulls: i64 = db
            .conn
            .query_row(
                "SELECT
                    (implied_speed_kmh IS NULL) + (last_fix_age_secs IS NULL) +
                    (inlier_ratio IS NULL) + (confidence IS NULL) + (suspect_reason IS NULL)
                 FROM fixes",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            quality_nulls, 5,
            "migration must not fabricate legacy quality"
        );
    }

    #[test]
    fn task0245_migrate_v6_to_v7_rolls_back_all_columns_then_retries() {
        let db = fresh_v6_db();
        db.conn
            .execute_batch(
                "CREATE TRIGGER reject_v7_stamp
                 BEFORE UPDATE ON schema_version
                 WHEN NEW.version = 7
                 BEGIN
                     SELECT RAISE(ABORT, 'injected v7 stamp failure');
                 END;",
            )
            .unwrap();

        let error = db.migrate_v6_to_v7().unwrap_err();
        assert!(format!("{error:#}").contains("injected v7 stamp failure"));
        assert_eq!(db.get_schema_version(), 6);
        for name in [
            "implied_speed_kmh",
            "last_fix_age_secs",
            "inlier_ratio",
            "confidence",
            "suspect_reason",
        ] {
            assert!(!db.column_exists("fixes", name), "{name} escaped rollback");
        }

        db.conn.execute("DROP TRIGGER reject_v7_stamp", []).unwrap();
        db.migrate_v6_to_v7().unwrap();
        assert_eq!(db.get_schema_version(), 7);
        assert!(db.fix_quality_columns_have_expected_shape());
    }

    #[test]
    fn task0245_v7_shape_repair_completes_every_individual_missing_column() {
        let expected = [
            ("implied_speed_kmh", "REAL"),
            ("last_fix_age_secs", "INTEGER"),
            ("inlier_ratio", "REAL"),
            ("confidence", "TEXT"),
            ("suspect_reason", "TEXT"),
        ];
        for omitted in expected.map(|(name, _)| name) {
            let db = fresh_v6_db();
            for (name, declared_type) in expected {
                if name != omitted {
                    db.conn
                        .execute(
                            &format!("ALTER TABLE fixes ADD COLUMN {name} {declared_type}"),
                            [],
                        )
                        .unwrap();
                }
            }
            db.conn
                .execute("UPDATE schema_version SET version = 7 WHERE id = 1", [])
                .unwrap();

            db.migrate().unwrap();

            // Shape repair resets the stamp down to the true v6 shape, then the
            // chain re-runs forward all the way to current (rebuilding the v7
            // quality bundle en route and applying v7->v8). The landing stamp
            // tracks SCHEMA_VERSION, not the literal v7 this test forced.
            assert_eq!(
                db.get_schema_version(),
                SCHEMA_VERSION,
                "failed repair omitting {omitted}"
            );
            assert!(
                db.fix_quality_columns_have_expected_shape(),
                "incomplete repair omitting {omitted}"
            );
        }
    }

    #[test]
    fn task0245_v7_partial_repair_preserves_existing_data_and_rejects_malformed_shape() {
        let db = fresh_v6_db();
        db.conn
            .execute("ALTER TABLE fixes ADD COLUMN implied_speed_kmh REAL", [])
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO fixes (
                    at_rfc3339, lat, lon, accuracy_m, n_sources, implied_speed_kmh
                 ) VALUES ('2026-07-05T12:00:00+00:00', 1, 2, 3, 4, 12.5)",
                [],
            )
            .unwrap();
        db.conn
            .execute("UPDATE schema_version SET version = 7 WHERE id = 1", [])
            .unwrap();
        db.migrate().unwrap();
        let preserved: f64 = db
            .conn
            .query_row("SELECT implied_speed_kmh FROM fixes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(preserved, 12.5);
        assert!(db.fix_quality_columns_have_expected_shape());

        let malformed = fresh_v6_db();
        malformed
            .conn
            .execute(
                "ALTER TABLE fixes ADD COLUMN confidence INTEGER DEFAULT 0",
                [],
            )
            .unwrap();
        malformed
            .conn
            .execute("UPDATE schema_version SET version = 7 WHERE id = 1", [])
            .unwrap();
        let error = malformed.migrate().unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("malformed fixes.confidence"), "{message}");
        assert!(
            message.contains("expected TEXT NULL with no default"),
            "{message}"
        );
    }

    #[test]
    fn task0245_future_schema_missing_quality_is_never_repaired_down() {
        let db = fresh_v6_db();
        db.conn
            .execute("UPDATE schema_version SET version = 99 WHERE id = 1", [])
            .unwrap();
        db.migrate().unwrap();
        assert_eq!(db.get_schema_version(), 99);
        assert!(!db.column_exists("fixes", "confidence"));
    }

    #[test]
    fn migrate_repairs_v6_stamp_missing_is_mobile_column() {
        let db = fresh_v2_db();
        db.migrate_v2_to_v3().unwrap();
        db.migrate_v3_to_v4().unwrap();
        db.migrate_v4_to_v5().unwrap();
        db.conn
            .execute("UPDATE schema_version SET version = 6 WHERE id = 1", [])
            .unwrap();
        assert!(!db.column_exists("aps", "is_mobile"));

        db.migrate().unwrap();

        // The stamp is repaired down to the true shape (v5, is_mobile absent),
        // then re-migrated forward all the way to current — restoring is_mobile
        // (v6) and adding the quality bundle (v7). So the landing stamp tracks
        // SCHEMA_VERSION, not the literal v6 this test was written against.
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);
        assert!(db.column_exists("aps", "is_mobile"));
    }

    /// task-0191 AC#3 guard rail: a genuinely-newer schema from a FUTURE
    /// binary (version > SCHEMA_VERSION) must NOT be repaired down, even
    /// though its unknown tables are "missing" from our point of view.
    /// Forward-compatibility: we leave it exactly as found.
    #[test]
    fn migrate_leaves_future_schema_untouched() {
        let db = Database::open_memory().unwrap();
        db.conn
            .execute("UPDATE schema_version SET version = 99 WHERE id = 1", [])
            .unwrap();
        db.migrate().unwrap();
        assert_eq!(
            db.get_schema_version(),
            99,
            "a version > SCHEMA_VERSION must be left untouched (no down-repair)"
        );
    }

    #[test]
    fn test_ap_crud() {
        let db = Database::open_memory().unwrap();
        let ap = ApInfo {
            bssid: "AA:BB:CC:DD:EE:FF".to_string(),
            ssid: Some("TestWiFi".to_string()),
            lat: 55.6684,
            lon: 12.5541,
            encryption: None,
            frequency: Some(2437),
            city: None,
            country: None,
            is_mobile: false,
            source: Source::Wigle,
        };
        db.upsert_ap(&ap).unwrap();
        let got = db.get_ap("AA:BB:CC:DD:EE:FF").unwrap().unwrap();
        assert_eq!(got.ssid, Some("TestWiFi".to_string()));
        assert!((got.lat - 55.6684).abs() < 1e-6);
        assert_eq!(got.source, Source::Wigle);
    }

    #[test]
    fn null_ssid_with_train_oui_is_not_mobile() {
        let db = Database::open_memory().unwrap();
        let mut ap = sample_ap(1);
        ap.bssid = "B8:50:01:AA:BB:CC".to_owned();
        ap.ssid = None;

        db.upsert_ap(&ap).unwrap();

        let stored = db.get_ap(&ap.bssid).unwrap().unwrap();
        assert!(!stored.is_mobile, "OUI alone must never classify mobility");
    }

    #[test]
    fn upsert_recomputes_mobile_flag_and_does_not_retroactively_reclassify() {
        let mut db = Database::open_memory_with_mobile_ssid_denylist(Vec::new()).unwrap();
        let mut ap = sample_ap(2);
        ap.ssid = Some("DSB_Wi-Fi".to_owned());
        ap.is_mobile = true; // ignored: persistence recomputes from its denylist
        db.upsert_ap(&ap).unwrap();
        assert!(!db.get_ap(&ap.bssid).unwrap().unwrap().is_mobile);

        db.mobile_ssid_denylist = crate::scanner::default_mobile_ssid_denylist();
        assert!(
            !db.get_ap(&ap.bssid).unwrap().unwrap().is_mobile,
            "changing the denylist must not rewrite existing rows"
        );

        db.upsert_ap(&ap).unwrap();
        assert!(
            db.get_ap(&ap.bssid).unwrap().unwrap().is_mobile,
            "the next actual upsert must use the current denylist"
        );

        ap.ssid = Some("fixed-home-network".to_owned());
        ap.is_mobile = true;
        db.upsert_ap(&ap).unwrap();
        assert!(
            !db.get_ap(&ap.bssid).unwrap().unwrap().is_mobile,
            "incoming flags are never trusted; a later fixed SSID clears the bit"
        );
    }

    /// Build an ApInfo with the given source and lat (used to tell rows
    /// apart in priority assertions).
    fn ap_with(source: Source, lat: f64) -> ApInfo {
        ApInfo {
            bssid: "AA:BB:CC:DD:EE:FF".to_string(),
            ssid: Some(format!("ssid-{}", source.as_str())),
            lat,
            lon: 12.5541,
            encryption: None,
            frequency: Some(2437),
            city: None,
            country: None,
            is_mobile: false,
            source,
        }
    }

    // source_priority_ladder test moved to db/source.rs alongside the enum.

    /// upsert_ap honours the priority ladder. Exercise every cross-source
    /// transition: equal-priority overwrites, higher-priority overwrites,
    /// lower-priority does NOT overwrite the data fields, but `last_seen`
    /// always advances (an observation is always recorded).
    #[test]
    fn upsert_ap_respects_source_priority() {
        let db = Database::open_memory().unwrap();
        let bssid = "AA:BB:CC:DD:EE:FF";

        // 1. Wigle writes first.
        db.upsert_ap(&ap_with(Source::Wigle, 1.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, Source::Wigle);
        assert!((got.lat - 1.0).abs() < 1e-9);

        // 2. Lower-priority manual write must NOT overwrite.
        db.upsert_ap(&ap_with(Source::Manual, 2.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, Source::Wigle, "manual must not overwrite wigle");
        assert!((got.lat - 1.0).abs() < 1e-9, "lat must not change");

        // 3. Lower-priority beacondb write must NOT overwrite wigle either.
        db.upsert_ap(&ap_with(Source::BeaconDb, 3.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, Source::Wigle);

        // 4. Equal-priority same-source rewrite SHOULD update the position
        //    (we trust the more recent observation from the same source).
        db.upsert_ap(&ap_with(Source::Wigle, 4.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, Source::Wigle);
        assert!(
            (got.lat - 4.0).abs() < 1e-9,
            "same-source rewrite should update lat, got {}",
            got.lat
        );

        // 5. Higher-priority apple write SHOULD overwrite.
        db.upsert_ap(&ap_with(Source::Apple, 5.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, Source::Apple);
        assert!((got.lat - 5.0).abs() < 1e-9);

        // 6. Subsequent lower-priority writes (wigle, beacondb, manual,
        //    unknown) must NOT overwrite apple.
        for (src, lat) in [
            (Source::Wigle, 6.0),
            (Source::BeaconDb, 7.0),
            (Source::Manual, 8.0),
            (Source::Unknown, 9.0),
        ] {
            db.upsert_ap(&ap_with(src, lat)).unwrap();
            let got = db.get_ap(bssid).unwrap().unwrap();
            assert_eq!(
                got.source,
                Source::Apple,
                "{:?} must not overwrite apple",
                src
            );
            assert!((got.lat - 5.0).abs() < 1e-9);
        }
    }

    #[test]
    fn lower_priority_upsert_refreshes_scanner_ssid_and_mobile_flag_only() {
        let db = Database::open_memory().unwrap();
        let mut apple = ap_with(Source::Apple, 55.0);
        apple.ssid = Some("fixed-home-network".to_owned());
        db.upsert_ap(&apple).unwrap();

        let mut manual_observation = ap_with(Source::Manual, 1.0);
        manual_observation.ssid = Some("MOVIA_BUS_42".to_owned());
        db.upsert_ap(&manual_observation).unwrap();

        let stored = db.get_ap(&apple.bssid).unwrap().unwrap();
        assert_eq!(stored.source, Source::Apple);
        assert_eq!(stored.lat, 55.0, "lower-priority position must not win");
        assert_eq!(stored.ssid.as_deref(), Some("MOVIA_BUS_42"));
        assert!(stored.is_mobile);
    }

    /// Migration v1 -> v2 backfills source_priority from the source string.
    /// We construct a v1 database by hand, then call `migrate` and verify
    /// the column exists and is populated correctly.
    #[test]
    fn migrate_v1_to_v2_backfills_source_priority() {
        let conn = Connection::open_in_memory().unwrap();
        // Build a v1 schema and seed it.
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version VALUES (1);
             CREATE TABLE aps (
                 bssid TEXT PRIMARY KEY, ssid TEXT, lat REAL NOT NULL, lon REAL NOT NULL,
                 encryption TEXT, channel INTEGER, frequency INTEGER, city TEXT, country TEXT,
                 source TEXT NOT NULL, first_seen TEXT NOT NULL, last_seen TEXT NOT NULL,
                 fetched_at TEXT NOT NULL
             );
             INSERT INTO aps VALUES ('A','x',1,1,NULL,NULL,NULL,NULL,NULL,'apple','t','t','t');
             INSERT INTO aps VALUES ('B','x',1,1,NULL,NULL,NULL,NULL,NULL,'wigle','t','t','t');
             INSERT INTO aps VALUES ('C','x',1,1,NULL,NULL,NULL,NULL,NULL,'beacondb','t','t','t');
             INSERT INTO aps VALUES ('D','x',1,1,NULL,NULL,NULL,NULL,NULL,'manual','t','t','t');
             INSERT INTO aps VALUES ('E','x',1,1,NULL,NULL,NULL,NULL,NULL,'mystery','t','t','t');
             CREATE TABLE not_found (bssid TEXT PRIMARY KEY, first_seen TEXT, last_seen TEXT, checked_at TEXT);
             CREATE TABLE pending (bssid TEXT PRIMARY KEY, ssid TEXT, channel INTEGER, frequency INTEGER, signal_dbm INTEGER, first_seen TEXT, last_seen TEXT, attempts INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        let db = Database {
            conn,
            mobile_ssid_denylist: crate::scanner::default_mobile_ssid_denylist(),
        };
        db.migrate().unwrap();
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);

        let priorities: Vec<(String, i32)> = {
            let mut stmt = db
                .conn
                .prepare("SELECT bssid, source_priority FROM aps ORDER BY bssid")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert_eq!(
            priorities,
            vec![
                ("A".to_string(), 40), // apple
                ("B".to_string(), 30), // wigle
                ("C".to_string(), 20), // beacondb
                ("D".to_string(), 10), // manual
                ("E".to_string(), 0),  // unknown
            ]
        );
    }

    #[test]
    fn test_not_found() {
        let db = Database::open_memory().unwrap();
        db.insert_not_found("AA:BB:CC:DD:EE:FF").unwrap();
        assert!(db.is_not_found("AA:BB:CC:DD:EE:FF", 30).unwrap());
        assert!(!db.is_not_found("11:22:33:44:55:66", 30).unwrap());
    }

    /// `prune_not_found` must delete rows older than 2×ttl while leaving
    /// still-live rows untouched. The AC nominally says 1000 stale rows, but a
    /// handful proves the DELETE identically and runs fast; the load-bearing
    /// assertion is the boundary — a fresh row (checked_at = now) must SURVIVE,
    /// so a prune that wiped the whole table would fail here (task-0110).
    #[test]
    fn prune_not_found_evicts_past_2x_ttl_and_spares_fresh() {
        let db = Database::open_memory().unwrap();
        let ttl_days = 30_i64;

        // Backdate a handful of rows to well past 2×ttl (75 days > 60) by
        // inserting a fixed old RFC3339 checked_at directly. Columns/format
        // mirror insert_not_found_on: (bssid, first_seen, last_seen, checked_at).
        let stale_checked_at = (Utc::now() - chrono::TimeDelta::days(75)).to_rfc3339();
        for i in 0..4 {
            let bssid = format!("AA:BB:CC:DD:EE:{i:02X}");
            db.execute_batch_for_test(&format!(
                "INSERT INTO not_found (bssid, first_seen, last_seen, checked_at)
                 VALUES ('{bssid}', '{stale_checked_at}', '{stale_checked_at}', '{stale_checked_at}');"
            ))
            .unwrap();
        }

        // A mid-age row at 45 days: expired past 1×ttl (30) but INSIDE 2×ttl
        // (60), so prune must SPARE it. This pins the `2 *` multiplier — if
        // prune used 1×ttl instead of 2×ttl this row would be deleted and the
        // assertions below (pruned==4, count==2) would fail (task-0110 review).
        let mid_checked_at = (Utc::now() - chrono::TimeDelta::days(45)).to_rfc3339();
        db.execute_batch_for_test(&format!(
            "INSERT INTO not_found (bssid, first_seen, last_seen, checked_at)
             VALUES ('99:99:99:99:99:99', '{mid_checked_at}', '{mid_checked_at}', '{mid_checked_at}');"
        ))
        .unwrap();

        // One fresh row within ttl: checked_at = now.
        db.insert_not_found("11:22:33:44:55:66").unwrap();
        assert_eq!(db.not_found_ap_count().unwrap(), 6);

        // Prune: only the 4 >2×ttl rows go; the 45-day (1×<age<2×) and the fresh
        // row both stay. pruned==4 (not 5) is what pins the 2×ttl boundary.
        let pruned = db.prune_not_found(ttl_days).unwrap();
        assert_eq!(pruned, 4, "only rows older than 2×ttl (60d) must be pruned");
        assert_eq!(
            db.not_found_ap_count().unwrap(),
            2,
            "the 45-day (past 1×ttl, within 2×ttl) row and the fresh row must both survive"
        );
        // The fresh row is still within ttl (queryable); the 45-day row survived
        // prune but is expired for is_not_found purposes (that is correct — it is
        // eligible for revival, not deletion, until it crosses 2×ttl).
        assert!(db.is_not_found("11:22:33:44:55:66", ttl_days).unwrap());
        assert!(!db.is_not_found("99:99:99:99:99:99", ttl_days).unwrap());
    }

    #[test]
    fn test_pending() {
        let db = Database::open_memory().unwrap();
        db.insert_pending("AA:BB:CC:DD:EE:FF", Some("Test"), Some(2437), Some(-65))
            .unwrap();
        let pending = db.get_pending(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].bssid, "AA:BB:CC:DD:EE:FF");
        assert_eq!(pending[0].ssid.as_deref(), Some("Test"));
        assert_eq!(pending[0].attempts, 0);

        db.increment_pending_attempts("AA:BB:CC:DD:EE:FF").unwrap();
        let pending = db.get_pending(10).unwrap();
        assert_eq!(pending[0].attempts, 1);

        db.delete_pending("AA:BB:CC:DD:EE:FF").unwrap();
        let pending = db.get_pending(10).unwrap();
        assert!(pending.is_empty());
    }

    /// `is_pending` must reflect the same state as `get_pending` but cheaply.
    /// We exercise the three transitions: insert, delete, and a never-seen
    /// BSSID. The indexed PRIMARY KEY probe means this stays O(log n) even
    /// with a large pending queue.
    #[test]
    fn is_pending_reflects_queue_membership() {
        let db = Database::open_memory().unwrap();
        let bssid = "AA:BB:CC:DD:EE:FF";
        let other = "11:22:33:44:55:66";

        // Empty queue: neither BSSID is pending.
        assert!(!db.is_pending(bssid).unwrap());
        assert!(!db.is_pending(other).unwrap());

        // After insert: only the inserted one is pending.
        db.insert_pending(bssid, Some("Test"), Some(2437), Some(-65))
            .unwrap();
        assert!(db.is_pending(bssid).unwrap());
        assert!(!db.is_pending(other).unwrap());

        // After delete: pending again returns false.
        db.delete_pending(bssid).unwrap();
        assert!(!db.is_pending(bssid).unwrap());
    }

    #[test]
    fn test_rate_limiting() {
        let mut db = Database::open_memory().unwrap();
        assert_eq!(db.wigle_lookups_today().unwrap(), 0);

        // First reservation under generous limit succeeds.
        assert_eq!(
            db.try_reserve_wigle_lookup(100).unwrap(),
            QuotaDecision::Reserved
        );
        assert_eq!(db.wigle_lookups_today().unwrap(), 1);

        // Limit-of-1 path: already at 1, so further reservation denied.
        assert_eq!(
            db.try_reserve_wigle_lookup(1).unwrap(),
            QuotaDecision::Exhausted
        );
        // Counter unchanged after a denied reservation.
        assert_eq!(db.wigle_lookups_today().unwrap(), 1);

        // Limit-of-2 path: still room for one more.
        assert_eq!(
            db.try_reserve_wigle_lookup(2).unwrap(),
            QuotaDecision::Reserved
        );
        assert_eq!(db.wigle_lookups_today().unwrap(), 2);

        // Refund returns the slot.
        db.refund_wigle_lookup().unwrap();
        assert_eq!(db.wigle_lookups_today().unwrap(), 1);

        // Refund clamps at 0.
        db.refund_wigle_lookup().unwrap();
        db.refund_wigle_lookup().unwrap();
        assert_eq!(db.wigle_lookups_today().unwrap(), 0);
    }

    /// task-0097: a corrupt `wigle_lookups_today` value must fail **closed** —
    /// `try_reserve_wigle_lookup` denies the slot and leaves the garbage row
    /// untouched, rather than healing it to 0 and re-charging the full daily
    /// WiGLE quota on top of whatever was already spent.
    #[test]
    fn try_reserve_fails_closed_on_corrupt_counter() {
        let mut db = Database::open_memory().unwrap();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        // Seed a corrupt counter for *today* so try_reserve reads it instead
        // of resetting on a date rollover.
        db.conn
            .execute(
                "INSERT INTO metadata (key, value) VALUES ('api_calls_date', ?1)",
                rusqlite::params![today],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO metadata (key, value) VALUES ('wigle_lookups_today', 'garbage')",
                [],
            )
            .unwrap();

        // Fail closed: deny the reservation despite a generous limit.
        let decision = db.try_reserve_wigle_lookup(100).unwrap();
        assert_eq!(
            decision,
            QuotaDecision::CounterCorrupt,
            "corrupt counter must be distinct from normal exhaustion"
        );

        // The corrupt row must NOT be healed/overwritten by the denied call.
        let raw: String = db
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'wigle_lookups_today'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(raw, "garbage", "corrupt counter row must be left untouched");
    }

    #[test]
    fn corrupt_api_counter_warnings_are_escaped_and_context_specific() {
        let raw = "bad\n\u{1b}";
        let escaped = format!("{raw:?}");

        let (_, parse_events) = capture_events(|| parse_wigle_lookups_value(Some(raw.to_owned())));
        assert!(
            parse_events.is_empty(),
            "the parser must remain side-effect-free"
        );

        let mut reservation_db = Database::open_memory().unwrap();
        seed_api_counter_for_today(&reservation_db, raw);
        let (decision, reservation_events) =
            capture_events(|| reservation_db.try_reserve_wigle_lookup(100).unwrap());
        assert_eq!(decision, QuotaDecision::CounterCorrupt);
        let reservation_warnings = warning_messages(&reservation_events);
        assert_eq!(reservation_warnings.len(), 1);
        assert!(reservation_warnings[0].contains("quota reservation denied"));
        assert!(reservation_warnings[0].contains(&escaped));
        assert!(!reservation_warnings[0].contains("refund"));
        assert!(!reservation_warnings[0].contains("stats reports"));

        let mut refund_db = Database::open_memory().unwrap();
        seed_api_counter_for_today(&refund_db, raw);
        let (refund_result, refund_events) = capture_events(|| refund_db.refund_wigle_lookup());
        assert!(refund_result.is_err());
        let refund_warnings = warning_messages(&refund_events);
        assert_eq!(refund_warnings.len(), 1);
        assert!(refund_warnings[0].contains("quota refund skipped"));
        assert!(refund_warnings[0].contains(&escaped));
        assert!(!refund_warnings[0].contains("reservation denied"));
        assert!(!refund_warnings[0].contains("stats reports"));

        let stats_db = Database::open_memory().unwrap();
        stats_db.set_metadata("wigle_lookups_today", raw).unwrap();
        let (stats_count, stats_events) =
            capture_events(|| stats_db.wigle_lookups_today_for_stats().unwrap());
        assert_eq!(stats_count, 0);
        let stats_warnings = warning_messages(&stats_events);
        assert_eq!(stats_warnings.len(), 1);
        assert!(stats_warnings[0].contains("stats reports wigle_lookups_today fallback 0"));
        assert!(stats_warnings[0].contains(&escaped));
        assert!(!stats_warnings[0].contains("reservation denied"));
        assert!(!stats_warnings[0].contains("refund skipped"));

        for warning in reservation_warnings
            .into_iter()
            .chain(refund_warnings)
            .chain(stats_warnings)
        {
            assert!(!warning.contains('\n'));
            assert!(!warning.contains('\u{1b}'));
        }
    }

    /// Exercise repeated reservations from many caller threads against one
    /// synchronous Database test fixture. The mutex here only makes the
    /// fixture shareable; production uses the writer actor, and the separate
    /// two-connection test above proves SQLite IMMEDIATE contention semantics.
    #[test]
    fn test_rate_limiting_concurrent() {
        use std::sync::{Arc, Mutex};
        use std::thread;

        let db = Arc::new(Mutex::new(Database::open_memory().unwrap()));
        let limit: u32 = 10;
        let threads = 64;

        let reserved = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let mut handles = Vec::new();
        for _ in 0..threads {
            let db = Arc::clone(&db);
            let reserved = Arc::clone(&reserved);
            handles.push(thread::spawn(move || {
                let ok = {
                    let mut g = db.lock().unwrap();
                    g.try_reserve_wigle_lookup(limit).unwrap()
                };
                if ok == QuotaDecision::Reserved {
                    reserved.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let total = reserved.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(total, limit, "total reservations must equal daily_limit");
        assert_eq!(db.lock().unwrap().wigle_lookups_today().unwrap(), limit);
    }

    #[test]
    fn last_fix_round_trip() {
        let db = Database::open_memory().unwrap();
        // Empty initially.
        assert!(db.get_last_fix().unwrap().is_none());

        let row = LastFix {
            lat: 55.6761,
            lon: 12.5683,
            accuracy_m: 42.0,
            address: Some("Copenhagen".to_string()),
            at: ts("2026-05-09T12:34:56+00:00"),
            sources: 5,
        };
        db.set_last_fix(&row).unwrap();
        let got = db.get_last_fix().unwrap().unwrap();
        assert!((got.lat - row.lat).abs() < 1e-9);
        assert!((got.lon - row.lon).abs() < 1e-9);
        assert!((got.accuracy_m - row.accuracy_m).abs() < 1e-9);
        assert_eq!(got.address, row.address);
        assert_eq!(got.at, row.at);
        assert_eq!(got.sources, row.sources);

        // Overwrite must work and stay single-row.
        let updated = LastFix {
            lat: 48.8566,
            lon: 2.3522,
            accuracy_m: 99.0,
            address: None,
            at: ts("2026-05-10T00:00:00+00:00"),
            sources: 7,
        };
        db.set_last_fix(&updated).unwrap();
        let got = db.get_last_fix().unwrap().unwrap();
        assert!((got.lat - 48.8566).abs() < 1e-9);
        assert_eq!(got.address, None);
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM last_fix", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "last_fix must remain single-row after overwrite");
    }

    /// task-0238 (relocated from `server::last_fix_rehydration_tests` in
    /// task-0134): `last_fix_from_columns` — the single Row->domain choke point
    /// `get_last_fix` funnels through — rejects a non-finite coordinate as a
    /// `CorruptLastFix`. Tested against the conversion directly because NaN/±Inf
    /// cannot round-trip through a SQLite REAL column (SQLite stores them NULL).
    #[test]
    fn last_fix_from_columns_discards_non_finite_coordinates() {
        let poisoned = [
            (f64::NAN, 12.5, "NaN latitude"),
            (f64::INFINITY, 12.5, "+Inf latitude"),
            (f64::NEG_INFINITY, 12.5, "-Inf latitude"),
            (55.6, f64::NAN, "NaN longitude"),
            (55.6, f64::INFINITY, "+Inf longitude"),
            (55.6, f64::NEG_INFINITY, "-Inf longitude"),
        ];
        for (lat, lon, case) in poisoned {
            let error = last_fix_from_columns(
                lat,
                lon,
                25.0,
                Some("Copenhagen".to_string()),
                "2026-07-04T12:00:00+00:00".to_string(),
                3,
            )
            .err()
            .unwrap_or_else(|| panic!("{case} must return Err"));
            assert!(
                error
                    .to_string()
                    .contains("non-finite last_fix coordinates"),
                "{case} error must name the root cause, got {error}"
            );
            assert!(
                error.downcast_ref::<CorruptLastFix>().is_some(),
                "{case} must be a CorruptLastFix (discard, not a read failure)"
            );
        }

        let finite = last_fix_from_columns(
            55.6,
            12.5,
            25.0,
            None,
            "2026-07-04T12:00:00+00:00".to_string(),
            3,
        )
        .expect("finite row must convert");
        assert_eq!(finite.lat, 55.6);
        assert_eq!(finite.lon, 12.5);
    }

    /// task-0238 (relocated in task-0134): non-finite or negative accuracy is a
    /// `CorruptLastFix`; zero accuracy remains valid.
    #[test]
    fn last_fix_from_columns_rejects_invalid_accuracy() {
        for accuracy_m in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01] {
            let error = last_fix_from_columns(
                55.6,
                12.5,
                accuracy_m,
                None,
                "2026-07-04T12:00:00+00:00".to_string(),
                3,
            )
            .expect_err("non-finite or negative accuracy must be rejected");
            assert!(error.to_string().contains("invalid last_fix accuracy_m"));
            assert!(error.downcast_ref::<CorruptLastFix>().is_some());
        }

        assert!(
            last_fix_from_columns(
                55.6,
                12.5,
                0.0,
                None,
                "2026-07-04T12:00:00+00:00".to_string(),
                3,
            )
            .is_ok(),
            "zero accuracy remains valid"
        );
    }

    /// task-0134 AC#2/#5: a corrupt `at_rfc3339` STRING round-trips through the
    /// SQLite TEXT column (unlike NaN), so this drives the real `get_last_fix`
    /// read path. It must return a typed `CorruptLastFix` error — NOT `Ok` with
    /// a silently-defaulted or stale timestamp.
    #[test]
    fn get_last_fix_on_corrupt_timestamp_is_err_not_stale() {
        let db = Database::open_memory().unwrap();
        // A valid row first, so a regression that ignores the corruption would
        // surface a stale-but-parseable value rather than an error.
        db.set_last_fix(&LastFix {
            lat: 55.0,
            lon: 12.0,
            accuracy_m: 10.0,
            address: None,
            at: ts("2026-07-04T12:00:00+00:00"),
            sources: 3,
        })
        .unwrap();
        // Overwrite the timestamp column in place with an unparseable value.
        db.execute_batch_for_test(
            "UPDATE last_fix SET at_rfc3339 = 'not-a-timestamp' WHERE id = 1;",
        )
        .unwrap();

        let error = db
            .get_last_fix()
            .expect_err("a corrupt persisted timestamp must be a typed read error");
        assert!(
            error.to_string().contains("unparseable last_fix timestamp"),
            "error must name the root cause, got {error}"
        );
        assert!(
            error.downcast_ref::<CorruptLastFix>().is_some(),
            "a corrupt persisted row must be a CorruptLastFix (discarded, not served)"
        );
    }

    #[test]
    fn last_fix_table_rejects_second_row() {
        // The CHECK (id = 1) constraint must reject any insert with id != 1.
        let db = Database::open_memory().unwrap();
        let row = LastFix {
            lat: 0.0,
            lon: 0.0,
            accuracy_m: 1.0,
            address: None,
            at: ts("2026-05-09T00:00:00+00:00"),
            sources: 0,
        };
        db.set_last_fix(&row).unwrap();
        let err = db.conn.execute(
            "INSERT INTO last_fix (id, lat, lon, accuracy_m, address, at_rfc3339, sources)
             VALUES (2, 0, 0, 0, NULL, '2026-05-09T00:00:00+00:00', 0)",
            [],
        );
        assert!(
            err.is_err(),
            "INSERT with id=2 must violate CHECK constraint, got {err:?}"
        );
    }

    /// task-0100: set_last_fix is monotonic by at_rfc3339. A write carrying an
    /// OLDER timestamp than the stored row (e.g. after the clock steps
    /// backwards) must be rejected with a warning, leaving the newer fix intact. A
    /// subsequent newer write must still apply (the guard is `>=`, not `==`).
    #[test]
    fn set_last_fix_drops_older_timestamp() {
        let db = Database::open_memory().unwrap();
        let t2 = LastFix {
            lat: 55.0,
            lon: 12.0,
            accuracy_m: 10.0,
            address: Some("newer".to_string()),
            at: ts("2026-05-10T00:00:00+00:00"),
            sources: 5,
        };
        db.set_last_fix(&t2).unwrap();

        // Older write (T1 < T2) must NOT overwrite the newer stored fix.
        let t1 = LastFix {
            lat: 48.0,
            lon: 2.0,
            accuracy_m: 99.0,
            address: Some("older".to_string()),
            at: ts("2026-05-09T00:00:00+00:00"),
            sources: 1,
        };
        crate::test_support::install_permissive_global_test_subscriber(); // task-0251
        let captured = crate::test_support::CaptureSink::default();
        let older_handle = captured.clone();
        install_commit_marker(&db, &captured);
        subscriber::with_default(captured, || db.set_last_fix(&t1).unwrap());
        db.conn.commit_hook(None::<fn() -> bool>);
        let older_events = to_level_message(older_handle.events());
        assert_one_last_fix_drop_warning(&older_events, &t1.at.to_rfc3339(), &t2.at.to_rfc3339());
        assert_commit_precedes_last_fix_warning(&older_events);

        let got = db.get_last_fix().unwrap().unwrap();
        assert_eq!(
            got.at.to_rfc3339(),
            "2026-05-10T00:00:00+00:00",
            "older write must be dropped"
        );
        assert!(
            (got.lat - 55.0).abs() < 1e-9,
            "position must remain the newer fix, got lat {}",
            got.lat
        );
        assert_eq!(got.address, Some("newer".to_string()));

        // Sanity: an equal-or-newer write still applies.
        let t3 = LastFix {
            lat: 10.0,
            lon: 20.0,
            accuracy_m: 5.0,
            address: Some("newest".to_string()),
            at: ts("2026-05-11T00:00:00+00:00"),
            sources: 9,
        };
        let (_, newer_events) = capture_events(|| db.set_last_fix(&t3).unwrap());
        assert!(warning_messages(&newer_events).is_empty());
        let got = db.get_last_fix().unwrap().unwrap();
        assert_eq!(got.at.to_rfc3339(), "2026-05-11T00:00:00+00:00");
        assert!((got.lat - 10.0).abs() < 1e-9);

        // Equal-timestamp write MUST apply: the address-backfill path
        // (server.rs) re-persists the same fix with only `address` changed
        // and an unchanged `at_rfc3339`. This pins the `>=` (not `>`) guard —
        // a regression to `>` would silently stop address backfill.
        let t3_backfill = LastFix {
            address: Some("backfilled".to_string()),
            ..t3.clone()
        };
        let (_, equal_events) = capture_events(|| db.set_last_fix(&t3_backfill).unwrap());
        assert!(warning_messages(&equal_events).is_empty());
        let got = db.get_last_fix().unwrap().unwrap();
        assert_eq!(
            got.address,
            Some("backfilled".to_string()),
            "equal-timestamp write must apply (address backfill depends on >=)"
        );
    }

    #[test]
    fn combined_older_fix_keeps_last_fix_appends_history_and_warns_after_commit() {
        let mut db = Database::open_memory().unwrap();
        let stored = LastFix {
            lat: 55.0,
            lon: 12.0,
            accuracy_m: 10.0,
            address: Some("stored".to_owned()),
            at: ts("2026-05-10T00:00:00+00:00"),
            sources: 5,
        };
        db.set_last_fix(&stored).unwrap();
        let incoming = LastFix {
            lat: 48.0,
            lon: 2.0,
            accuracy_m: 99.0,
            address: Some("incoming".to_owned()),
            at: ts("2026-05-09T00:00:00+00:00"),
            sources: 1,
        };

        crate::test_support::install_permissive_global_test_subscriber(); // task-0251
        let captured = crate::test_support::CaptureSink::default();
        let handle = captured.clone();
        install_commit_marker(&db, &captured);
        subscriber::with_default(captured, || {
            db.persist_fix_and_history(&incoming, &test_quality())
                .unwrap()
        });
        db.conn.commit_hook(None::<fn() -> bool>);
        let events = to_level_message(handle.events());
        assert_one_last_fix_drop_warning(
            &events,
            &incoming.at.to_rfc3339(),
            &stored.at.to_rfc3339(),
        );
        assert_commit_precedes_last_fix_warning(&events);
        let retained = db.get_last_fix().unwrap().unwrap();
        assert_eq!(retained.at, stored.at);
        assert_eq!(retained.address, stored.address);
        let history = db
            .get_fixes_in_range("2026-05-08T00:00:00+00:00", "2026-05-11T00:00:00+00:00")
            .unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].at, incoming.at);
        assert_eq!(
            history[0].quality.as_ref(),
            Some(&test_quality()),
            "backward-clock history must retain incoming observation quality"
        );
    }

    #[test]
    fn combined_older_fix_history_failure_rolls_back_without_warning() {
        let mut db = Database::open_memory().unwrap();
        let stored = LastFix {
            lat: 55.0,
            lon: 12.0,
            accuracy_m: 10.0,
            address: Some("stored".to_owned()),
            at: ts("2026-05-10T00:00:00+00:00"),
            sources: 5,
        };
        db.set_last_fix(&stored).unwrap();
        db.conn
            .execute_batch(
                "CREATE TRIGGER reject_fix_history
                 BEFORE INSERT ON fixes
                 BEGIN
                   SELECT RAISE(ABORT, 'injected history failure');
                 END;",
            )
            .unwrap();
        let incoming = LastFix {
            lat: 48.0,
            lon: 2.0,
            accuracy_m: 99.0,
            address: Some("incoming".to_owned()),
            at: ts("2026-05-09T00:00:00+00:00"),
            sources: 1,
        };

        let (result, events) =
            capture_events(|| db.persist_fix_and_history(&incoming, &test_quality()));
        assert!(result.is_err());
        assert!(
            warning_messages(&events)
                .into_iter()
                .all(|message| !message.contains("persisted last_fix update rejected")),
            "rolled-back operation must emit no drop warning: {events:?}"
        );
        let retained = db.get_last_fix().unwrap().unwrap();
        assert_eq!(retained.at, stored.at);
        let history_count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM fixes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(history_count, 0);
    }

    /// task-0047: a migration body that returns Err must roll back the
    /// transaction, leaving the DB at the prior schema version with no
    /// partial DDL applied.
    #[test]
    fn migration_rollback_on_error() {
        let db = Database::open_memory().unwrap();
        // Baseline: fully migrated DB at SCHEMA_VERSION.
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);

        // Snapshot the table list pre-attempt.
        let tables_before: Vec<String> = {
            let mut stmt = db
                .conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
            rows.map(|r| r.unwrap()).collect()
        };

        // Run a migration body that creates a table then errors. The CREATE
        // must NOT survive after the helper returns.
        let result: Result<()> = db.run_migration(|tx| {
            tx.execute_batch("CREATE TABLE rollback_target (id INTEGER PRIMARY KEY);")?;
            anyhow::bail!("simulated mid-migration failure");
        });
        assert!(result.is_err(), "expected helper to surface body error");

        let tables_after: Vec<String> = {
            let mut stmt = db
                .conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
            rows.map(|r| r.unwrap()).collect()
        };
        assert_eq!(
            tables_before, tables_after,
            "rollback_target should not exist after failed migration"
        );
        // schema_version still at SCHEMA_VERSION (never bumped).
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn fixes_round_trip_and_range_query() {
        let db = Database::open_memory().unwrap();
        // Three fixes spread across a day.
        db.insert_fix("2026-05-09T08:00:00+00:00", 55.0, 12.0, 30.0, 5)
            .unwrap();
        db.insert_fix("2026-05-09T12:00:00+00:00", 55.001, 12.001, 25.0, 6)
            .unwrap();
        db.insert_fix("2026-05-09T16:00:00+00:00", 55.0, 12.0, 35.0, 4)
            .unwrap();

        // Query whole day: should return all 3 in ascending order.
        let all = db
            .get_fixes_in_range("2026-05-09T00:00:00+00:00", "2026-05-09T23:59:59+00:00")
            .unwrap();
        assert_eq!(all.len(), 3);
        assert!(all.iter().all(|row| row.quality.is_none()));
        assert_eq!(all[0].at.to_rfc3339(), "2026-05-09T08:00:00+00:00");
        assert_eq!(all[2].at.to_rfc3339(), "2026-05-09T16:00:00+00:00");

        // Narrow query to the middle hour: only the 12:00 fix.
        let mid = db
            .get_fixes_in_range("2026-05-09T11:00:00+00:00", "2026-05-09T13:00:00+00:00")
            .unwrap();
        assert_eq!(mid.len(), 1);
        assert!((mid[0].lat - 55.001).abs() < 1e-9);
    }

    #[test]
    fn task0245_quality_round_trips_through_full_and_capped_queries() {
        let mut db = Database::open_memory().unwrap();
        db.insert_fix("2026-07-05T11:59:00+00:00", 1.0, 2.0, 3.0, 4)
            .unwrap();
        let quality = FixQuality::new(
            Some(123.5),
            Some(42),
            Some(0.8),
            Confidence::Suspect,
            Some("exact policy reason".to_owned()),
        )
        .unwrap();
        db.persist_fix_and_history(
            &LastFix {
                lat: 55.0,
                lon: 12.0,
                accuracy_m: 9.0,
                address: None,
                at: ts("2026-07-05T12:00:00+00:00"),
                sources: 5,
            },
            &quality,
        )
        .unwrap();

        let full = db.get_fixes_in_range("0000", "9999").unwrap();
        assert_eq!(full.len(), 2);
        assert_eq!(full[0].quality, None, "legacy all-NULL row is unknown");
        assert_eq!(full[1].quality.as_ref(), Some(&quality));

        let capped = db.get_capped_fixes_in_range("0000", "9999", 1).unwrap();
        assert_eq!(capped.elided, Some(1));
        assert_eq!(capped.fixes.len(), 1);
        assert_eq!(capped.fixes[0].quality.as_ref(), Some(&quality));
    }

    #[test]
    fn task0245_quality_zero_and_ratio_boundaries_round_trip() {
        let mut db = Database::open_memory().unwrap();
        for (index, ratio) in [0.0, 1.0].into_iter().enumerate() {
            let quality =
                FixQuality::new(Some(0.0), Some(0), Some(ratio), Confidence::High, None).unwrap();
            db.persist_fix_and_history(
                &LastFix {
                    lat: 1.0,
                    lon: 2.0,
                    accuracy_m: 3.0,
                    address: None,
                    at: ts(&format!("2026-07-05T12:00:0{index}+00:00")),
                    sources: 4,
                },
                &quality,
            )
            .unwrap();
        }
        let rows = db.get_fixes_in_range("0000", "9999").unwrap();
        assert_eq!(rows[0].quality.as_ref().unwrap().inlier_ratio(), Some(0.0));
        assert_eq!(rows[1].quality.as_ref().unwrap().inlier_ratio(), Some(1.0));
        assert!(rows.iter().all(|row| {
            let quality = row.quality.as_ref().unwrap();
            quality.implied_speed_kmh() == Some(0.0) && quality.last_fix_age_secs() == Some(0)
        }));
    }

    #[test]
    fn task0245_corrupt_quality_never_degrades_to_legacy_unknown() {
        let db = Database::open_memory().unwrap();
        let cases = [
            ("-1, 1, NULL, 'high', NULL", "implied speed"),
            ("1e999, 1, NULL, 'high', NULL", "implied speed"),
            ("1, NULL, NULL, 'high', NULL", "present together"),
            ("NULL, NULL, -0.1, 'low', NULL", "inlier ratio"),
            ("NULL, NULL, 1.1, 'high', NULL", "inlier ratio"),
            ("NULL, NULL, 1e999, 'high', NULL", "inlier ratio"),
            ("NULL, NULL, NULL, 'future', NULL", "unknown confidence"),
            ("NULL, NULL, NULL, 'high', 'wrong'", "only for suspect"),
            (
                "NULL, NULL, NULL, 'suspect', NULL",
                "requires a nonempty reason",
            ),
            (
                "NULL, NULL, NULL, 'suspect', ''",
                "requires a nonempty reason",
            ),
            (
                "NULL, NULL, 0.5, NULL, NULL",
                "without the required confidence",
            ),
        ];
        for (values, expected) in cases {
            db.conn.execute("DELETE FROM fixes", []).unwrap();
            db.conn
                .execute_batch(&format!(
                    "INSERT INTO fixes (
                        at_rfc3339, lat, lon, accuracy_m, n_sources,
                        implied_speed_kmh, last_fix_age_secs, inlier_ratio,
                        confidence, suspect_reason
                     ) VALUES ('2026-07-05T12:00:00+00:00', 1, 2, 3, 4, {values});"
                ))
                .unwrap();
            let row_id: i64 = db
                .conn
                .query_row("SELECT id FROM fixes", [], |row| row.get(0))
                .unwrap();
            let error = db.get_fixes_in_range("0000", "9999").unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains(&format!("row id {row_id}")), "{message}");
            assert!(message.contains(expected), "{message}");
        }

        db.conn.execute("DELETE FROM fixes", []).unwrap();
        db.conn
            .execute_batch(
                "INSERT INTO fixes (
                    at_rfc3339, lat, lon, accuracy_m, n_sources, confidence, inlier_ratio
                 ) VALUES ('2026-07-05T12:00:00+00:00', 1, 2, 3, 4, 'high', 2.0);",
            )
            .unwrap();
        let error = db.get_capped_fixes_in_range("0000", "9999", 1).unwrap_err();
        assert!(format!("{error:#}").contains("corrupt fixes row id"));
    }

    #[test]
    fn capped_fixes_select_newest_then_return_ascending_with_exact_elision() {
        let db = Database::open_memory().unwrap();
        for index in 0..1_001 {
            db.insert_fix(
                &format!("2026-05-09T00:{:02}:{:02}+00:00", index / 60, index % 60),
                55.0 + index as f64 / 100_000.0,
                12.0,
                5.0,
                index,
            )
            .unwrap();
        }

        for (limit, expected_first, expected_elided) in
            [(999, 2, Some(2)), (1_000, 1, Some(1)), (1_001, 0, None)]
        {
            let result = db
                .get_capped_fixes_in_range(
                    "2026-05-09T00:00:00+00:00",
                    "2026-05-09T23:59:59+00:00",
                    limit,
                )
                .unwrap();
            assert_eq!(result.fixes.len(), limit as usize);
            assert_eq!(result.fixes[0].n_sources, expected_first);
            assert_eq!(result.fixes.last().unwrap().n_sources, 1_000);
            assert_eq!(result.elided, expected_elided);
        }
    }

    #[test]
    fn capped_fixes_bounds_and_duplicate_timestamps_use_id_tie_break() {
        let db = Database::open_memory().unwrap();
        db.insert_fix("2026-05-09T00:00:00+00:00", 0.0, 0.0, 1.0, 1)
            .unwrap();
        for lat in [1.0, 2.0, 3.0] {
            db.insert_fix("2026-05-09T12:00:00+00:00", lat, 0.0, 1.0, 1)
                .unwrap();
        }
        db.insert_fix("2026-05-10T00:00:00+00:00", 4.0, 0.0, 1.0, 1)
            .unwrap();

        let inclusive = db
            .get_capped_fixes_in_range("2026-05-09T00:00:00+00:00", "2026-05-10T00:00:00+00:00", 5)
            .unwrap();
        assert_eq!(
            inclusive
                .fixes
                .iter()
                .map(|row| row.lat)
                .collect::<Vec<_>>(),
            vec![0.0, 1.0, 2.0, 3.0, 4.0]
        );
        assert_eq!(inclusive.elided, None);

        let newest_duplicates = db
            .get_capped_fixes_in_range("2026-05-09T12:00:00+00:00", "2026-05-09T12:00:00+00:00", 2)
            .unwrap();
        assert_eq!(
            newest_duplicates
                .fixes
                .iter()
                .map(|row| row.lat)
                .collect::<Vec<_>>(),
            vec![2.0, 3.0],
            "newest duplicate ids must be retained, then returned id-ascending"
        );
        assert_eq!(newest_duplicates.elided, Some(1));
    }

    #[test]
    fn capped_fixes_count_and_page_are_indexed() {
        let db = Database::open_memory().unwrap();
        for (sql, parameters) in [
            (
                COUNT_FIXES_IN_RANGE_SQL,
                vec!["2026-01-01T00:00:00+00:00", "2026-12-31T23:59:59+00:00"],
            ),
            (
                SELECT_NEWEST_FIXES_IN_RANGE_SQL,
                vec![
                    "2026-01-01T00:00:00+00:00",
                    "2026-12-31T23:59:59+00:00",
                    "1000",
                ],
            ),
        ] {
            let mut statement = db
                .conn
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap();
            let details = statement
                .query_map(rusqlite::params_from_iter(parameters), |row| {
                    row.get::<_, String>(3)
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert!(
                details.iter().any(|detail| {
                    detail.contains("SEARCH fixes") && detail.contains("idx_fixes_at")
                }),
                "raw history query must search idx_fixes_at: {details:?}"
            );
        }
    }

    #[test]
    fn capped_fixes_count_and_page_share_one_read_snapshot() {
        let path = std::env::temp_dir().join(format!(
            "whereamid_raw_history_snapshot_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let reader = Database::open(&path).unwrap();
        let writer = Database::open(&path).unwrap();
        reader
            .insert_fix("2026-05-09T00:00:00+00:00", 1.0, 2.0, 3.0, 1)
            .unwrap();

        let result = reader
            .get_capped_fixes_in_range_with_hook(
                "2026-05-09T00:00:00+00:00",
                "2026-05-09T23:59:59+00:00",
                10,
                || {
                    writer
                        .insert_fix("2026-05-09T01:00:00+00:00", 4.0, 5.0, 6.0, 1)
                        .unwrap();
                },
            )
            .unwrap();
        assert_eq!(result.fixes.len(), 1);
        assert_eq!(result.elided, None);
        assert_eq!(
            reader
                .get_fixes_in_range("2026-05-09T00:00:00+00:00", "2026-05-09T23:59:59+00:00")
                .unwrap()
                .len(),
            2,
            "writer row must exist after the snapshot transaction ends"
        );

        drop(writer);
        drop(reader);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn fixes_prune_drops_old_rows() {
        let db = Database::open_memory().unwrap();
        // One row 100 days old, one row "now".
        let old_ts = (chrono::Utc::now() - chrono::TimeDelta::days(100)).to_rfc3339();
        let new_ts = chrono::Utc::now().to_rfc3339();
        db.insert_fix(&old_ts, 0.0, 0.0, 1.0, 1).unwrap();
        db.insert_fix(&new_ts, 1.0, 1.0, 1.0, 1).unwrap();

        let pruned = db.prune_fixes(30).unwrap();
        assert_eq!(pruned, 1, "expected 1 row pruned");

        let remaining = db
            .get_fixes_in_range("2000-01-01T00:00:00+00:00", "2099-01-01T00:00:00+00:00")
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert!((remaining[0].lat - 1.0).abs() < 1e-9);
    }

    /// task-0101: retention_days == 0 disables pruning (keep forever). It must
    /// be a no-op even when every row is far older than "now" — the previous
    /// behaviour computed cutoff == now and deleted them all on every sweep.
    #[test]
    fn prune_fixes_zero_retention_is_noop() {
        let db = Database::open_memory().unwrap();
        let old_ts = (chrono::Utc::now() - chrono::TimeDelta::days(365)).to_rfc3339();
        for _ in 0..10 {
            db.insert_fix(&old_ts, 1.0, 2.0, 3.0, 1).unwrap();
        }

        let pruned = db.prune_fixes(0).unwrap();
        assert_eq!(pruned, 0, "retention 0 must delete nothing");

        let remaining = db
            .get_fixes_in_range("2000-01-01T00:00:00+00:00", "2099-01-01T00:00:00+00:00")
            .unwrap();
        assert_eq!(
            remaining.len(),
            10,
            "no fixes should be deleted with retention 0"
        );

        // Defense-in-depth: a negative retention must be rejected before SQL,
        // not converted into a future cutoff that could wipe live history.
        assert!(db.prune_fixes(-5).is_err());
        let remaining = db
            .get_fixes_in_range("2000-01-01T00:00:00+00:00", "2099-01-01T00:00:00+00:00")
            .unwrap();
        assert_eq!(
            remaining.len(),
            10,
            "negative retention must not wipe history"
        );
    }

    #[test]
    fn last_fix_survives_db_reopen() {
        // Round-trip through an on-disk DB so we exercise the file-backed path.
        // Avoid adding a tempfile dep — manual cleanup is fine for one test.
        let path = std::env::temp_dir().join(format!(
            "whereamid_last_fix_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let row = LastFix {
            lat: 51.5074,
            lon: -0.1278,
            accuracy_m: 25.0,
            address: Some("London".to_string()),
            at: ts("2026-05-09T08:00:00+00:00"),
            sources: 3,
        };
        let result = (|| -> Result<()> {
            {
                let db = Database::open(&path)?;
                db.set_last_fix(&row)?;
            }
            // Reopen: rehydration source-of-truth is on disk.
            let db = Database::open(&path)?;
            let got = db
                .get_last_fix()?
                .expect("rehydrated row must be present after reopen");
            assert!((got.lat - row.lat).abs() < 1e-9);
            assert!((got.lon - row.lon).abs() < 1e-9);
            assert_eq!(got.address, row.address);
            Ok(())
        })();
        // Cleanup the .db file plus WAL/SHM siblings even on failure.
        for ext in ["", "-wal", "-shm"] {
            let mut p = path.clone();
            if !ext.is_empty() {
                p.set_file_name(format!(
                    "{}{}",
                    p.file_name().unwrap().to_string_lossy(),
                    ext
                ));
            }
            let _ = std::fs::remove_file(&p);
        }
        result.unwrap();
    }

    // --- task-0197: DB file permission hardening -------------------------

    /// Unique throwaway directory for one file-backed test (no `tempfile`
    /// dependency — matches tests/shutdown.rs's convention).
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

    /// "<path>-wal" / "<path>-shm" as a sibling PathBuf (append, do NOT replace
    /// the .sqlite extension).
    fn sidecar(path: &Path, suffix: &str) -> std::path::PathBuf {
        let mut s = path.as_os_str().to_owned();
        s.push(suffix);
        s.into()
    }

    fn mode_bits(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// task-0197 AC#2: a freshly created DB file is owner-only (0600). (The
    /// 0700 state-directory half lives in main.rs's ensure_private_dir test —
    /// Database::open does not create the parent directory.)
    #[test]
    fn open_creates_db_file_0600() {
        let dir = unique_tempdir("perms-fresh");
        let path = dir.join("aps.sqlite");
        {
            let _db = Database::open(&path).unwrap();
        }
        assert_eq!(
            mode_bits(&path),
            0o600,
            "a freshly created DB file must be 0600, not group/other readable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// task-0197 AC#3: an existing DB left group/other-readable by an older
    /// daemon is tightened back to 0600 on the next open.
    #[test]
    fn open_tightens_preexisting_loose_db() {
        use std::os::unix::fs::PermissionsExt;
        let dir = unique_tempdir("perms-existing");
        let path = dir.join("aps.sqlite");
        // First open creates a valid, migrated DB (already 0600).
        {
            let _db = Database::open(&path).unwrap();
        }
        // Simulate an older daemon: loosen the file to world-readable 0644.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(mode_bits(&path), 0o644, "precondition: DB is loose");
        // Reopen: startup must tighten it back.
        {
            let _db = Database::open(&path).unwrap();
        }
        assert_eq!(
            mode_bits(&path),
            0o600,
            "an existing loose DB must be tightened to 0600 on open"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// task-0197: the tighten helper covers the main file AND pre-existing
    /// -wal/-shm sidecars, and silently ignores a missing sidecar (NotFound).
    /// Exercised directly (deterministically) because a clean connection close
    /// checkpoints and removes the sidecars, making them awkward to observe
    /// via the integration path.
    #[test]
    fn tighten_db_file_permissions_covers_main_and_sidecars() {
        use std::os::unix::fs::PermissionsExt;
        let dir = unique_tempdir("perms-tighten");
        let path = dir.join("aps.sqlite");
        let wal = sidecar(&path, "-wal");
        let shm = sidecar(&path, "-shm");
        // Create main + both sidecars, all group/world readable.
        for p in [&path, &wal, &shm] {
            std::fs::write(p, b"x").unwrap();
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        tighten_db_file_permissions(&path);
        for p in [&path, &wal, &shm] {
            assert_eq!(
                mode_bits(p),
                0o600,
                "{} must be tightened to 0600",
                p.display()
            );
        }
        // A missing sidecar must be a silent no-op (no panic).
        std::fs::remove_file(&wal).unwrap();
        tighten_db_file_permissions(&path);
        assert_eq!(mode_bits(&path), 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// task-0197 review follow-up: prove the chmod-before-WAL ordering claim
    /// against REAL SQLite behavior — a -wal sidecar created by SQLite itself
    /// (not hand-made by the test) must be born without group/other bits,
    /// because SQLite copies the main DB file's mode (0600 after the tighten
    /// in open()). Observed while the connection is still open: a clean close
    /// checkpoints and deletes the sidecar. Under a 0077 umask this passes
    /// trivially; under the common 0022 it is exactly the inheritance that
    /// keeps the sidecar owner-only.
    #[test]
    fn real_wal_sidecar_is_born_owner_only() {
        let dir = unique_tempdir("perms-walborn");
        let path = dir.join("aps.sqlite");
        let wal = sidecar(&path, "-wal");
        {
            let db = Database::open(&path).unwrap();
            // open() already writes through WAL (schema creation), but pin an
            // explicit post-open write so the sidecar's existence is not an
            // artifact of migration internals.
            db.set_metadata("walborn-probe", "1").unwrap();
            assert!(
                wal.exists(),
                "-wal sidecar must exist while the connection is open"
            );
            assert_eq!(
                mode_bits(&wal) & 0o077,
                0,
                "SQLite-created -wal must not be group/other accessible (mode {:o})",
                mode_bits(&wal)
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
