//! Bounded, latest-only background Nominatim scheduling (task-0193).
//!
//! Locates never spawn geocode tasks. They synchronously publish one desired
//! rounded grid cell into a single pending slot; one named worker owns all
//! rate waiting and HTTP work. A generation watch cancels work superseded
//! before transport, while the exact-key active set coalesces same-cell jitter.

#[cfg(test)]
use super::lock_db;
use super::{address_cache_key, backfill_last_fix_address_with, DaemonState};
use crate::nominatim::REQUEST_INTERVAL;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tokio::time::{Duration, Instant};
use tracing::warn;

type GridKey = (i32, i32);

/// Allow one complete policy-mandated rate wait plus explicit scheduling
/// tolerance. Superseded generations are rejected immediately; this bound is
/// for current work delayed before dispatch, not for response processing.
const QUEUE_SCHEDULING_TOLERANCE: Duration = Duration::from_secs(1);
const MAX_QUEUE_AGE: Duration = REQUEST_INTERVAL.saturating_add(QUEUE_SCHEDULING_TOLERANCE);

#[derive(Clone, Copy, Debug)]
struct Candidate {
    key: GridKey,
    lat: f64,
    lon: f64,
    enqueued_at: Instant,
    generation: u64,
    privacy_ticket: crate::privacy::PrivacyTicket,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActiveToken {
    generation: u64,
    privacy_ticket: crate::privacy::PrivacyTicket,
}

#[derive(Default)]
struct SchedulerState {
    pending: Option<Candidate>,
    active: HashMap<GridKey, ActiveToken>,
    latest_key: Option<GridKey>,
    latest_needs_lookup: bool,
    generation: u64,
    worker_running: bool,
}

/// One-worker, one-pending-slot reverse-geocode scheduler.
pub(crate) struct GeocodeScheduler {
    state: Mutex<SchedulerState>,
    changed: watch::Sender<u64>,
    shutdown: tokio_util::sync::CancellationToken,
}

impl GeocodeScheduler {
    pub(crate) fn new(shutdown: tokio_util::sync::CancellationToken) -> Self {
        let (changed, _) = watch::channel(0);
        Self {
            state: Mutex::new(SchedulerState::default()),
            changed,
            shutdown,
        }
    }

    /// Observe the address-cache result for the current locate.
    ///
    /// Callers hold the address-cache mutex while invoking this method. That
    /// cache -> scheduler ordering makes the hit/miss observation atomic with
    /// worker insertion, and the worker uses the same ordering. The scheduler
    /// never acquires the cache itself and never holds its mutex across await.
    pub(super) fn observe(
        &self,
        lat: f64,
        lon: f64,
        needs_lookup: bool,
        privacy_ticket: crate::privacy::PrivacyTicket,
    ) -> bool {
        if self.shutdown.is_cancelled() {
            return false;
        }

        let Some(key) = address_cache_key(lat, lon) else {
            // A non-finite position has no cache identity and must never reach
            // Nominatim. It still supersedes older queued work.
            self.supersede_without_work(None);
            return false;
        };

        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.shutdown.is_cancelled() {
            return false;
        }

        if !needs_lookup {
            let changed = state.pending.is_some()
                || !state.active.is_empty()
                || state.latest_key != Some(key)
                || state.latest_needs_lookup;
            state.pending = None;
            state.latest_key = Some(key);
            state.latest_needs_lookup = false;
            if changed {
                let generation = bump_generation(&mut state);
                self.changed.send_replace(generation);
            }
            drop(state);
            return false;
        }

        // A queued same-cell observation updates the exact coordinates and
        // age in place without creating another claim or generation. An
        // already-active same-cell operation is the dedupe boundary.
        if let Some(pending) = state.pending.as_mut().filter(|p| p.key == key) {
            pending.lat = lat;
            pending.lon = lon;
            pending.enqueued_at = Instant::now();
            return false;
        }
        if state.active.contains_key(&key)
            && state.latest_key == Some(key)
            && state.latest_needs_lookup
        {
            return false;
        }

        let generation = bump_generation(&mut state);
        state.latest_key = Some(key);
        state.latest_needs_lookup = true;
        state.pending = Some(Candidate {
            key,
            lat,
            lon,
            enqueued_at: Instant::now(),
            generation,
            privacy_ticket,
        });
        self.changed.send_replace(generation);
        drop(state);
        true
    }

    fn supersede_without_work(&self, key: Option<GridKey>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.shutdown.is_cancelled() {
            return;
        }
        state.pending = None;
        state.latest_key = key;
        state.latest_needs_lookup = false;
        let generation = bump_generation(&mut state);
        self.changed.send_replace(generation);
        drop(state);
    }

    fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn take_pending(&self) -> Option<Candidate> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .take()
    }

    fn claim(&self, candidate: Candidate) -> Option<ActiveClaim<'_>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.active.is_empty() || state.active.contains_key(&candidate.key) {
            return None;
        }
        let token = ActiveToken {
            generation: candidate.generation,
            privacy_ticket: candidate.privacy_ticket,
        };
        state.active.insert(candidate.key, token);
        Some(ActiveClaim {
            scheduler: self,
            key: candidate.key,
            token,
        })
    }

    /// Whether this generation is still the latest requested position.
    /// Response validity uses only this predicate: network latency after a
    /// request was issued does not make an otherwise-current result stale.
    fn is_latest(&self, candidate: Candidate) -> bool {
        if self.shutdown.is_cancelled() {
            return false;
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.generation == candidate.generation
            && state.latest_key == Some(candidate.key)
            && state.latest_needs_lookup
    }

    /// Whether current work is still young enough to invoke transport.
    fn is_fresh_for_dispatch(&self, candidate: Candidate) -> bool {
        candidate.enqueued_at.elapsed() <= MAX_QUEUE_AGE && self.is_latest(candidate)
    }

    fn begin_worker(&self) -> Option<WorkerGuard<'_>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.worker_running {
            return None;
        }
        state.worker_running = true;
        Some(WorkerGuard { scheduler: self })
    }

    fn clear_pending(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.pending = None;
        state.latest_needs_lookup = false;
        let generation = bump_generation(&mut state);
        self.changed.send_replace(generation);
        drop(state);
    }

    /// Invalidate queued/active coordinates and wake the worker. The active
    /// claim is tokenized, so its eventual Drop cannot remove a same-key claim
    /// created after the wipe.
    pub(super) fn clear_for_wipe(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.pending = None;
        state.active.clear();
        state.latest_key = None;
        state.latest_needs_lookup = false;
        let generation = bump_generation(&mut state);
        self.changed.send_replace(generation);
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self) -> SchedulerSnapshot {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        SchedulerSnapshot {
            worker_running: usize::from(state.worker_running),
            active: state.active.len(),
            pending: usize::from(state.pending.is_some()),
        }
    }
}

fn bump_generation(state: &mut SchedulerState) -> u64 {
    state.generation = state.generation.wrapping_add(1);
    state.generation
}

struct ActiveClaim<'a> {
    scheduler: &'a GeocodeScheduler,
    key: GridKey,
    token: ActiveToken,
}

impl Drop for ActiveClaim<'_> {
    fn drop(&mut self) {
        let mut state = self
            .scheduler
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.active.get(&self.key) == Some(&self.token) {
            state.active.remove(&self.key);
        }
    }
}

struct WorkerGuard<'a> {
    scheduler: &'a GeocodeScheduler,
}

impl Drop for WorkerGuard<'_> {
    fn drop(&mut self) {
        let mut state = self
            .scheduler
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        state.worker_running = false;
        // Any worker exit is terminal for this worker instance. Clear queued
        // lookup state even on panic/unwind so no orphaned candidate remains
        // with `worker_running == false` and nobody able to consume it.
        state.pending = None;
        state.latest_needs_lookup = false;
    }
}

/// Wait until a notification corresponds to state that actually supersedes
/// `candidate`. Scheduler mutation and normal publication are serialized by
/// the state mutex, but treating state as authoritative also makes the worker
/// robust to a delayed/stale watch value instead of discarding the newest
/// pending candidate merely because the notification payload regressed.
async fn wait_for_supersession(
    scheduler: &GeocodeScheduler,
    changed: &mut watch::Receiver<u64>,
    candidate: Candidate,
) {
    loop {
        if changed.changed().await.is_err() {
            return;
        }
        let _observed = *changed.borrow_and_update();
        if !scheduler.is_latest(candidate) {
            return;
        }
    }
}

/// Run the scheduler's sole named worker until daemon shutdown.
pub(crate) async fn run_worker(state: Arc<DaemonState>) {
    let Some(_worker_guard) = state.geocode.begin_worker() else {
        warn!("refusing to start a second Nominatim geocode worker");
        return;
    };
    let mut changed = state.geocode.subscribe();

    loop {
        if state.shutdown.is_cancelled() {
            state.geocode.clear_pending();
            break;
        }

        let Some(candidate) = state.geocode.take_pending() else {
            tokio::select! {
                biased;
                _ = state.shutdown.cancelled() => {
                    state.geocode.clear_pending();
                    break;
                }
                result = changed.changed() => {
                    if result.is_err() {
                        break;
                    }
                }
            }
            continue;
        };

        let Some(_claim) = state.geocode.claim(candidate) else {
            warn!("geocode scheduler invariant violated: worker attempted a second active claim");
            continue;
        };

        // Acknowledge the generation that led to this candidate. If a newer
        // observation raced between take_pending and here, dispatch freshness
        // rejects it at the rate gate even though this receiver is caught up.
        let _observed = *changed.borrow_and_update();
        if !state.geocode.is_fresh_for_dispatch(candidate) {
            continue;
        }

        let result = tokio::select! {
            biased;
            _ = state.shutdown.cancelled() => None,
            _ = wait_for_supersession(&state.geocode, &mut changed, candidate) => None,
            result = state.nominatim.reverse_geocode_if(candidate.lat, candidate.lon, || {
                state.geocode.is_fresh_for_dispatch(candidate)
            }) => Some(result),
        };

        let Some(result) = result else {
            continue;
        };
        let address = match result {
            Ok(Some(address)) => address,
            Ok(None) => continue,
            Err(error) => {
                warn!("background reverse geocode failed: {error}");
                continue;
            }
        };

        // Lock order is cache -> scheduler, matching locate observation. The
        // freshness check and cache insertion are therefore one linearized
        // operation relative to every newer hit/miss observation.
        let inserted = {
            let Some(_privacy) = state.privacy.read(candidate.privacy_ticket).await else {
                continue;
            };
            let mut cache = state.address_cache.lock().unwrap_or_else(|e| {
                warn!("address cache mutex poisoned on insert — recovering");
                e.into_inner()
            });
            if state.geocode.is_latest(candidate) {
                cache.insert(candidate.lat, candidate.lon, address.display.clone());
                true
            } else {
                false
            }
        };
        if !inserted || !state.geocode.is_latest(candidate) {
            continue;
        }

        // Backfill only a still-current matching last_fix. The helper retains
        // the reviewed last_fix -> durable writer FIFO order from task-0046.
        if let Err(error) = backfill_last_fix_address_with(
            &state,
            candidate.privacy_ticket,
            candidate.lat,
            candidate.lon,
            &address.display,
            |db, row| async move { db.set_last_fix(row).await },
        )
        .await
        {
            warn!("failed to persist last_fix address backfill: {error}");
            state.record_db_failure();
        }
    }
}

#[cfg(test)]
#[derive(Debug)]
pub(super) struct SchedulerSnapshot {
    pub(super) worker_running: usize,
    pub(super) active: usize,
    pub(super) pending: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nominatim::{Address, NominatimClient};
    use crate::server::{persist_locate_fix_and_schedule, test_state_with_nominatim, LastFix};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::{mpsc, Semaphore};

    fn address(display: impl Into<String>) -> Address {
        Address {
            display: display.into(),
            road: None,
            house_number: None,
            city: None,
            postcode: None,
            country: None,
        }
    }

    async fn bounded_yields(mut condition: impl FnMut() -> bool) -> bool {
        for _ in 0..2_000 {
            if condition() {
                return true;
            }
            tokio::task::yield_now().await;
        }
        condition()
    }

    async fn worker_finished(mut handle: tokio::task::JoinHandle<()>) -> bool {
        match tokio::time::timeout(Duration::from_secs(2), &mut handle).await {
            Ok(Ok(())) => true,
            Ok(Err(_join_error)) => false,
            Err(_) => {
                // Cleanup remains bounded even when the shutdown contract
                // regresses: abort, then reap so no detached test worker leaks.
                handle.abort();
                let _ = handle.await;
                false
            }
        }
    }

    fn set_last_fix(state: &Arc<DaemonState>, lat: f64, lon: f64) {
        let mut last = state
            .last_fix
            .try_lock()
            .expect("last_fix unexpectedly busy");
        *last = Some(LastFix {
            lat,
            lon,
            accuracy_m: 5.0,
            address: None,
            at: chrono::Utc::now(),
            sources: 3,
        });
    }

    fn observe(state: &Arc<DaemonState>, lat: f64, lon: f64) -> Option<String> {
        let cache = state
            .address_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let cached = cache.get(lat, lon);
        state
            .geocode
            .observe(lat, lon, cached.is_none(), state.privacy.ticket());
        cached
    }

    /// AC#1: exercise the production cache+schedule seam concurrently. Raw
    /// sub-cell jitter shares one exact rounded key and therefore one active
    /// HashSet claim and one below-rate-gate transport invocation.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_locate_observations_for_one_cell_issue_one_request() {
        const LAT: f64 = 55.668412;
        const LON: f64 = 12.554123;

        let calls = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Semaphore::new(0));
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
        let client = NominatimClient::with_transport({
            let calls = Arc::clone(&calls);
            let release = Arc::clone(&release);
            move |lat, lon| {
                let calls = Arc::clone(&calls);
                let release = Arc::clone(&release);
                let entered_tx = entered_tx.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let _ = entered_tx.send((lat, lon));
                    let permit = release.acquire().await.expect("release semaphore closed");
                    permit.forget();
                    Ok(address("same-cell address"))
                }
            }
        });
        let state = test_state_with_nominatim(client);
        set_last_fix(&state, LAT, LON);
        let worker = tokio::spawn(run_worker(Arc::clone(&state)));

        // Establish one active request first. The following burst therefore
        // exercises the active-key HashSet boundary, not merely replacement
        // of a pending slot before the worker starts.
        persist_locate_fix_and_schedule(
            &state,
            state.privacy.ticket(),
            LastFix {
                lat: LAT,
                lon: LON,
                accuracy_m: 5.0,
                address: None,
                at: chrono::Utc::now(),
                sources: 3,
            },
        )
        .await;
        let entered = tokio::time::timeout(Duration::from_secs(2), entered_rx.recv())
            .await
            .ok()
            .flatten();

        let mut submissions = tokio::task::JoinSet::new();
        for i in 0..128 {
            let state = Arc::clone(&state);
            submissions.spawn(async move {
                // Both coordinates remain in the same 4-decimal cell.
                let jitter = if i % 2 == 0 { 0.000_003 } else { -0.000_003 };
                persist_locate_fix_and_schedule(
                    &state,
                    state.privacy.ticket(),
                    LastFix {
                        lat: LAT + jitter,
                        lon: LON + jitter,
                        accuracy_m: 5.0,
                        address: None,
                        at: chrono::Utc::now(),
                        sources: 3,
                    },
                )
                .await;
            });
        }
        while submissions.join_next().await.is_some() {}

        let while_blocked = state.geocode.snapshot();
        let duplicate_before_release = entered_rx.try_recv().ok();
        release.add_permits(1);
        let populated = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let cache_ready = state
                    .address_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(LAT, LON)
                    .is_some();
                let fix_ready = state
                    .last_fix
                    .lock()
                    .await
                    .as_ref()
                    .and_then(|fix| fix.address.as_ref())
                    .is_some();
                if cache_ready && fix_ready {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        let memory_address = state
            .last_fix
            .lock()
            .await
            .as_ref()
            .and_then(|fix| fix.address.clone());
        state.shutdown.cancel();
        let stopped = worker_finished(worker).await;

        assert!(
            entered.is_some(),
            "the sole request never reached transport"
        );
        assert!(
            duplicate_before_release.is_none(),
            "duplicate transport started"
        );
        assert_eq!(while_blocked.active, 1);
        assert_eq!(
            while_blocked.pending, 0,
            "same-cell observations must not queue behind an active request"
        );
        assert!(populated, "successful transport never populated the cache");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(memory_address.as_deref(), Some("same-cell address"));
        assert!(stopped, "worker did not stop after shutdown");
        let snapshot = state.geocode.snapshot();
        assert_eq!(snapshot.active, 0);
        assert_eq!(snapshot.pending, 0);
        assert_eq!(snapshot.worker_running, 0);
    }

    /// AC#2: one blocked transport plus thousands of moving positions still
    /// owns exactly one named worker, one active key, and one latest pending
    /// key. The detached-task JoinSet remains empty.
    #[tokio::test]
    async fn blocked_transport_with_thousands_of_moves_stays_hard_bounded() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
        let client = NominatimClient::with_transport({
            let calls = Arc::clone(&calls);
            move |lat, lon| {
                let calls = Arc::clone(&calls);
                let entered_tx = entered_tx.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let _ = entered_tx.send((lat, lon));
                    std::future::pending::<anyhow::Result<Address>>().await
                }
            }
        });
        let state = test_state_with_nominatim(client);
        let worker = tokio::spawn(run_worker(Arc::clone(&state)));
        observe(&state, 50.0, 10.0);
        let entered = tokio::time::timeout(Duration::from_secs(2), entered_rx.recv())
            .await
            .ok()
            .flatten();

        // No await in this burst: the active request remains blocked while
        // every intermediate position is synchronously overwritten.
        for i in 0..10_000 {
            observe(&state, 51.0 + i as f64 * 0.001, 11.0);
        }
        let bounded = state.geocode.snapshot();
        let tracked_tasks = state.tasks.lock().unwrap_or_else(|e| e.into_inner()).len();
        state.shutdown.cancel();
        let stopped = worker_finished(worker).await;
        let clean = state.geocode.snapshot();

        assert!(entered.is_some(), "initial request never reached transport");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(bounded.worker_running, 1);
        assert_eq!(bounded.active, 1);
        assert!(bounded.pending <= 1);
        assert!(bounded.active + bounded.pending <= 2);
        assert_eq!(tracked_tasks, 0, "locates must not grow the JoinSet");
        assert!(stopped);
        assert_eq!(clean.active, 0);
        assert_eq!(clean.pending, 0);
        assert_eq!(clean.worker_running, 0);
    }

    /// Model the exact former publication race deterministically: generation
    /// 2 owns the newest pending coordinates, then a delayed generation-1
    /// watch value arrives. Scheduler state is authoritative, so the worker
    /// must still issue only the generation-2 candidate.
    #[tokio::test]
    async fn delayed_older_notification_cannot_discard_newest_distinct_candidate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (transport_tx, mut transport_rx) = mpsc::unbounded_channel();
        let client = NominatimClient::with_transport({
            let calls = Arc::clone(&calls);
            move |lat, lon| {
                let calls = Arc::clone(&calls);
                let transport_tx = transport_tx.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let _ = transport_tx.send((lat, lon));
                    Ok(address("newest"))
                }
            }
        });
        let state = test_state_with_nominatim(client);
        observe(&state, 55.0, 12.0);
        observe(&state, 57.0, 14.0);
        let newest_generation = state
            .geocode
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .expect("newest candidate missing")
            .generation;

        // Emulate T1 publishing after T2 despite T2 having installed the
        // newer state. Production now publishes under the state lock, but the
        // worker also rejects stale notification payloads defensively.
        state
            .geocode
            .changed
            .send_replace(newest_generation.wrapping_sub(1));
        let worker = tokio::spawn(run_worker(Arc::clone(&state)));
        let invoked = tokio::time::timeout(Duration::from_secs(2), transport_rx.recv())
            .await
            .ok()
            .flatten();
        state.shutdown.cancel();
        let stopped = worker_finished(worker).await;

        assert_eq!(invoked, Some((57.0, 14.0)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(stopped);
    }

    /// AC#3: A is already waiting inside the real one-second limiter; B is
    /// overwritten by C. Advancing paused time releases the gate, but only C
    /// reaches the injected transport below it.
    #[tokio::test(start_paused = true)]
    async fn rate_waiting_work_is_superseded_and_only_latest_reaches_transport() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (transport_tx, mut transport_rx) = mpsc::unbounded_channel();
        let client = NominatimClient::with_transport({
            let calls = Arc::clone(&calls);
            move |lat, lon| {
                let calls = Arc::clone(&calls);
                let transport_tx = transport_tx.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let _ = transport_tx.send((lat, lon));
                    Ok(address(format!("{lat},{lon}")))
                }
            }
        });
        let state = test_state_with_nominatim(client);

        // Seed last_request at virtual t=0 so A must wait inside the limiter.
        let seed = state.nominatim.reverse_geocode(1.0, 1.0).await;
        let seeded_transport = transport_rx.recv().await;
        let worker = tokio::spawn(run_worker(Arc::clone(&state)));
        observe(&state, 55.0, 12.0); // A
        let a_active = bounded_yields(|| state.geocode.snapshot().active == 1).await;
        observe(&state, 56.0, 13.0); // B, overwritten
        observe(&state, 57.0, 14.0); // C, latest
                                     // Wake after the exact policy deadline, with realistic scheduling
                                     // slop. Current work must remain dispatchable inside the explicit
                                     // queue-age tolerance.
        tokio::time::advance(REQUEST_INTERVAL + Duration::from_millis(250)).await;
        let c_invoked = bounded_yields(|| calls.load(Ordering::SeqCst) == 2).await;
        let latest = transport_rx.try_recv().ok();
        state.shutdown.cancel();
        let stopped = worker_finished(worker).await;

        assert!(seed.is_ok());
        assert_eq!(seeded_transport, Some((1.0, 1.0)));
        assert!(a_active, "A was never claimed by the worker");
        assert!(c_invoked, "latest candidate never reached transport");
        assert_eq!(latest, Some((57.0, 14.0)));
        assert_eq!(calls.load(Ordering::SeqCst), 2, "A or B reached transport");
        assert!(stopped);
    }

    /// A cache hit at the new current position is still a supersession event:
    /// it must cancel an already-issued older request's response handling and
    /// preserve both the new cached address and the new last_fix row.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn newer_cache_hit_cancels_inflight_stale_result_without_mutation() {
        const OLD_LAT: f64 = 55.0;
        const OLD_LON: f64 = 12.0;
        const NEW_LAT: f64 = 56.0;
        const NEW_LON: f64 = 13.0;

        let calls = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Semaphore::new(0));
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
        let client = NominatimClient::with_transport({
            let calls = Arc::clone(&calls);
            let release = Arc::clone(&release);
            move |lat, lon| {
                let calls = Arc::clone(&calls);
                let release = Arc::clone(&release);
                let entered_tx = entered_tx.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let _ = entered_tx.send((lat, lon));
                    let permit = release.acquire().await.expect("release semaphore closed");
                    permit.forget();
                    Ok(address("stale address"))
                }
            }
        });
        let state = test_state_with_nominatim(client);
        set_last_fix(&state, OLD_LAT, OLD_LON);
        let worker = tokio::spawn(run_worker(Arc::clone(&state)));
        observe(&state, OLD_LAT, OLD_LON);
        let entered = tokio::time::timeout(Duration::from_secs(2), entered_rx.recv())
            .await
            .ok()
            .flatten();

        {
            let mut cache = state
                .address_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            cache.insert(NEW_LAT, NEW_LON, "current cached address".to_string());
        }
        let returned = persist_locate_fix_and_schedule(
            &state,
            state.privacy.ticket(),
            LastFix {
                lat: NEW_LAT,
                lon: NEW_LON,
                accuracy_m: 4.0,
                address: None,
                at: chrono::Utc::now(),
                sources: 4,
            },
        )
        .await;
        let stale_cancelled = tokio::time::timeout(Duration::from_secs(2), async {
            while state.geocode.snapshot().active != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();

        // Cleanup before assertions. A removed waiter leaves this permit
        // unused; if cancellation regresses, it releases the fake transport
        // so the worker can still terminate instead of hanging the suite.
        release.add_permits(1);
        state.shutdown.cancel();
        let stopped = worker_finished(worker).await;
        let old_cache = state
            .address_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(OLD_LAT, OLD_LON);
        let memory_row = {
            let last = state.last_fix.lock().await;
            crate::db::LastFixRow::from(last.as_ref().expect("last_fix missing"))
        };
        let disk_row = lock_db(&state)
            .get_last_fix()
            .unwrap()
            .expect("persisted last_fix missing");

        assert_eq!(entered, Some((OLD_LAT, OLD_LON)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(stale_cancelled, "new cache hit did not cancel stale work");
        assert!(stopped);
        assert_eq!(old_cache, None, "stale result mutated the old cache cell");
        assert_eq!(returned.as_deref(), Some("current cached address"));
        assert_eq!(memory_row.lat, NEW_LAT);
        assert_eq!(memory_row.lon, NEW_LON);
        assert_eq!(
            memory_row.address.as_deref(),
            Some("current cached address")
        );
        assert_eq!(disk_row.lat, memory_row.lat);
        assert_eq!(disk_row.lon, memory_row.lon);
        assert_eq!(disk_row.address, memory_row.address);
    }

    /// The generation can remain current while queue age alone expires. The
    /// final check below the rate gate rejects that stale position.
    #[tokio::test(start_paused = true)]
    async fn queue_age_bound_is_enforced_at_transport_boundary() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = NominatimClient::with_transport({
            let calls = Arc::clone(&calls);
            move |_lat, _lon| {
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(address("address"))
                }
            }
        });
        let state = test_state_with_nominatim(client);
        state.nominatim.reverse_geocode(1.0, 1.0).await.unwrap();
        let worker = tokio::spawn(run_worker(Arc::clone(&state)));
        observe(&state, 55.0, 12.0);
        let active = bounded_yields(|| state.geocode.snapshot().active == 1).await;
        tokio::time::advance(MAX_QUEUE_AGE + Duration::from_millis(1)).await;
        let cleared = bounded_yields(|| state.geocode.snapshot().active == 0).await;
        state.shutdown.cancel();
        let stopped = worker_finished(worker).await;

        assert!(active);
        assert!(cleared);
        assert!(stopped);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "expired work reached transport"
        );
        assert!(state
            .address_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(55.0, 12.0)
            .is_none());
    }

    /// Queue age limits only pre-dispatch work. Once transport was invoked
    /// while current, an unsuperseded response remains valid even when normal
    /// network latency extends beyond that dispatch budget.
    #[tokio::test(start_paused = true)]
    async fn slow_unsuperseded_success_survives_queue_age_after_dispatch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Semaphore::new(0));
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
        let client = NominatimClient::with_transport({
            let calls = Arc::clone(&calls);
            let release = Arc::clone(&release);
            move |lat, lon| {
                let calls = Arc::clone(&calls);
                let release = Arc::clone(&release);
                let entered_tx = entered_tx.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let _ = entered_tx.send((lat, lon));
                    let permit = release.acquire().await.expect("release semaphore closed");
                    permit.forget();
                    Ok(address("slow but current"))
                }
            }
        });
        let state = test_state_with_nominatim(client);
        set_last_fix(&state, 55.0, 12.0);
        let worker = tokio::spawn(run_worker(Arc::clone(&state)));
        observe(&state, 55.0, 12.0);
        let entered = tokio::time::timeout(Duration::from_secs(2), entered_rx.recv())
            .await
            .ok()
            .flatten();

        tokio::time::advance(MAX_QUEUE_AGE + Duration::from_secs(1)).await;
        release.add_permits(1);
        let populated = bounded_yields(|| {
            state
                .address_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(55.0, 12.0)
                .is_some()
        })
        .await;
        let memory_address = state
            .last_fix
            .lock()
            .await
            .as_ref()
            .and_then(|fix| fix.address.clone());
        state.shutdown.cancel();
        let stopped = worker_finished(worker).await;

        assert_eq!(entered, Some((55.0, 12.0)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            populated,
            "slow current response was discarded as queue-stale"
        );
        assert_eq!(memory_address.as_deref(), Some("slow but current"));
        assert!(stopped);
    }

    /// A failed request drops its RAII claim. A later observation of the same
    /// exact key can therefore retry instead of being permanently deduped.
    #[tokio::test(start_paused = true)]
    async fn failure_clears_claim_and_later_same_key_retries() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = NominatimClient::with_transport({
            let calls = Arc::clone(&calls);
            move |_lat, _lon| {
                let calls = Arc::clone(&calls);
                async move {
                    let attempt = calls.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        anyhow::bail!("injected failure");
                    }
                    Ok(address("retry succeeded"))
                }
            }
        });
        let state = test_state_with_nominatim(client);
        let worker = tokio::spawn(run_worker(Arc::clone(&state)));
        observe(&state, 55.0, 12.0);
        let first_cleared = bounded_yields(|| {
            calls.load(Ordering::SeqCst) == 1 && state.geocode.snapshot().active == 0
        })
        .await;
        let retried = observe(&state, 55.0, 12.0).is_none();
        let second_active = bounded_yields(|| state.geocode.snapshot().active == 1).await;
        tokio::time::advance(REQUEST_INTERVAL).await;
        let populated = bounded_yields(|| {
            state
                .address_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(55.0, 12.0)
                .is_some()
        })
        .await;
        state.shutdown.cancel();
        let stopped = worker_finished(worker).await;

        assert!(first_cleared);
        assert!(retried);
        assert!(second_active);
        assert!(populated);
        assert!(stopped);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(state.geocode.snapshot().active, 0);
    }

    /// Cancellation before submission rejects work; cancellation during a
    /// blocked transport drops the active claim and cannot start queued work.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_rejects_and_cleans_work_without_later_transport() {
        let before_calls = Arc::new(AtomicUsize::new(0));
        let before_client = NominatimClient::with_transport({
            let before_calls = Arc::clone(&before_calls);
            move |_lat, _lon| {
                let before_calls = Arc::clone(&before_calls);
                async move {
                    before_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(address("impossible"))
                }
            }
        });
        let before = test_state_with_nominatim(before_client);
        before.shutdown.cancel();
        let accepted_before = before
            .geocode
            .observe(55.0, 12.0, true, before.privacy.ticket());
        let before_worker = tokio::spawn(run_worker(Arc::clone(&before)));
        let before_stopped = worker_finished(before_worker).await;

        let during_calls = Arc::new(AtomicUsize::new(0));
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
        let during_client = NominatimClient::with_transport({
            let during_calls = Arc::clone(&during_calls);
            move |lat, lon| {
                let during_calls = Arc::clone(&during_calls);
                let entered_tx = entered_tx.clone();
                async move {
                    during_calls.fetch_add(1, Ordering::SeqCst);
                    let _ = entered_tx.send((lat, lon));
                    std::future::pending::<anyhow::Result<Address>>().await
                }
            }
        });
        let during = test_state_with_nominatim(during_client);
        let during_worker = tokio::spawn(run_worker(Arc::clone(&during)));
        observe(&during, 55.0, 12.0);
        let entered = tokio::time::timeout(Duration::from_secs(2), entered_rx.recv())
            .await
            .ok()
            .flatten();
        observe(&during, 56.0, 13.0);
        during.shutdown.cancel();
        let during_stopped = worker_finished(during_worker).await;
        let accepted_after = during
            .geocode
            .observe(57.0, 14.0, true, during.privacy.ticket());
        let clean = during.geocode.snapshot();

        assert!(!accepted_before);
        assert!(before_stopped);
        assert_eq!(before_calls.load(Ordering::SeqCst), 0);
        assert!(entered.is_some());
        assert!(during_stopped);
        assert!(!accepted_after);
        assert_eq!(during_calls.load(Ordering::SeqCst), 1);
        assert_eq!(clean.active, 0);
        assert_eq!(clean.pending, 0);
        assert_eq!(clean.worker_running, 0);
    }

    #[tokio::test]
    async fn pre_wipe_active_drop_cannot_remove_post_wipe_same_cell_claim() {
        let state = test_state_with_nominatim(NominatimClient::new());
        let old_ticket = state.privacy.ticket();
        assert!(state.geocode.observe(55.0, 12.0, true, old_ticket));
        let old_candidate = state.geocode.take_pending().unwrap();
        let old_claim = state.geocode.claim(old_candidate).unwrap();

        let mut wipe = state.privacy.begin_wipe().await;
        wipe.advance_epoch();
        state.geocode.clear_for_wipe();
        drop(wipe);

        let new_ticket = state.privacy.ticket();
        assert!(state.geocode.observe(55.0, 12.0, true, new_ticket));
        let new_candidate = state.geocode.take_pending().unwrap();
        let new_claim = state.geocode.claim(new_candidate).unwrap();
        drop(old_claim);
        assert_eq!(
            state.geocode.snapshot().active,
            1,
            "old claim Drop must retain the post-wipe same-cell claim"
        );
        drop(new_claim);
        assert_eq!(state.geocode.snapshot().active, 0);
    }

    /// A panic inside the selected transport future unwinds through both RAII
    /// guards. Even when a newer candidate was installed while that future
    /// was being polled, no active or pending state may outlive the worker.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn transport_panic_clears_active_and_pending_worker_state() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(std::sync::Mutex::new(Some(release_rx)));
        let client = NominatimClient::with_transport({
            let release_rx = Arc::clone(&release_rx);
            move |lat, lon| {
                let entered_tx = entered_tx.clone();
                let release_rx = Arc::clone(&release_rx);
                async move {
                    let _ = entered_tx.send((lat, lon));
                    let receiver = release_rx
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .take()
                        .expect("panic transport invoked more than once");
                    let _ = receiver.recv_timeout(std::time::Duration::from_secs(2));
                    panic!("injected transport panic");
                }
            }
        });
        let state = test_state_with_nominatim(client);
        let mut worker = tokio::spawn(run_worker(Arc::clone(&state)));
        observe(&state, 55.0, 12.0);
        let entered = entered_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .ok();
        observe(&state, 56.0, 13.0);
        let before_panic = state.geocode.snapshot();

        // Cleanup signal precedes all assertions. The transport panics in the
        // same poll after this unblocks, before select can choose the already-
        // ready generation notification.
        let _ = release_tx.send(());
        let panicked = match tokio::time::timeout(Duration::from_secs(2), &mut worker).await {
            Ok(Err(join_error)) => join_error.is_panic(),
            Ok(Ok(())) => false,
            Err(_) => {
                worker.abort();
                let _ = worker.await;
                false
            }
        };
        state.shutdown.cancel();
        let clean = state.geocode.snapshot();

        assert_eq!(entered, Some((55.0, 12.0)));
        assert_eq!(before_panic.active, 1);
        assert_eq!(before_panic.pending, 1);
        assert!(panicked, "transport panic was not reported by JoinHandle");
        assert_eq!(clean.active, 0);
        assert_eq!(clean.pending, 0);
        assert_eq!(clean.worker_running, 0);
    }

    /// Success always populates its cache cell, but task-0046 backfill may
    /// update only a last_fix whose rounded identity still matches.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn success_backfills_only_matching_last_fix() {
        let matching = test_state_with_nominatim(NominatimClient::with_transport(|_, _| async {
            Ok(address("matching"))
        }));
        set_last_fix(&matching, 55.0, 12.0);
        let matching_worker = tokio::spawn(run_worker(Arc::clone(&matching)));
        observe(&matching, 55.0, 12.0);
        let matching_done = tokio::time::timeout(Duration::from_secs(2), async {
            while matching
                .last_fix
                .lock()
                .await
                .as_ref()
                .and_then(|fix| fix.address.as_ref())
                .is_none()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        matching.shutdown.cancel();
        let matching_stopped = worker_finished(matching_worker).await;

        let mismatching =
            test_state_with_nominatim(NominatimClient::with_transport(|_, _| async {
                Ok(address("cache only"))
            }));
        set_last_fix(&mismatching, 40.0, 8.0);
        let mismatching_worker = tokio::spawn(run_worker(Arc::clone(&mismatching)));
        observe(&mismatching, 55.0, 12.0);
        let mismatch_cached = tokio::time::timeout(Duration::from_secs(2), async {
            while mismatching
                .address_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(55.0, 12.0)
                .is_none()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        let mismatch_address = mismatching
            .last_fix
            .lock()
            .await
            .as_ref()
            .and_then(|fix| fix.address.clone());
        mismatching.shutdown.cancel();
        let mismatching_stopped = worker_finished(mismatching_worker).await;

        assert!(matching_done);
        assert!(matching_stopped);
        assert!(mismatch_cached);
        assert_eq!(mismatch_address, None);
        assert!(mismatching_stopped);
    }
}
