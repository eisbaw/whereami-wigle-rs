//! Provider abstraction for BSSID -> ApInfo lookup.
//!
//! Each provider knows how to query a single backend (Apple WPS, WiGLE) for
//! one BSSID and report a uniform outcome. Providers handle their own
//! metering (e.g., recording a WiGLE API call against the daily quota) and
//! their own preconditions (e.g., rate-limit check, credentials configured).
//!
//! Providers do NOT touch the local cache, the not_found table, or the
//! pending queue — those side-effects are policy decisions that belong to
//! the chain orchestrator (see `resolver::resolve_chain`).
//!
//! The `Provider` enum gives static dispatch over a fixed set of backends
//! without pulling in `async_trait` or boxing every call.

use std::sync::Arc;
use tracing::warn;

use crate::db::ApInfo;
use crate::server::DaemonState;
use crate::wigle::WigleError;

/// Result of a single provider lookup. Application policy (caching, pending
/// queue, not_found marking) is applied by the orchestrator based on this.
pub enum ProviderOutcome {
    /// Provider returned a position for this BSSID.
    Found(ApInfo),
    /// Provider authoritatively says this BSSID is not in its database.
    NotFound,
    /// Provider was not consulted (rate-limited, not configured, etc.).
    /// The chain may try the next provider for the same BSSID.
    Skipped,
    /// A non-recoverable condition for the rest of this chain pass (e.g.,
    /// WiGLE rate-limited). The orchestrator stops calling this provider
    /// for any further BSSIDs in this pass.
    HardStop,
    /// Transient network error. The orchestrator may queue/retry per policy.
    NetworkError(anyhow::Error),
}

/// Static-dispatch enum over the available providers.
pub enum Provider {
    Apple,
    Wigle,
    /// Test-only mock that returns prebaked outcomes via a closure.
    /// Allows unit tests of `resolve_chain` to exercise every branch
    /// without making real network calls.
    #[cfg(test)]
    Mock(std::sync::Arc<MockProvider>),
}

impl Provider {
    pub fn name(&self) -> &'static str {
        match self {
            Provider::Apple => "apple",
            Provider::Wigle => "wigle",
            #[cfg(test)]
            Provider::Mock(m) => m.name,
        }
    }

    /// Look up a single BSSID via this provider. Performs the provider's
    /// own preconditions and metering. Never touches caches/pending tables.
    pub async fn lookup(&self, state: &Arc<DaemonState>, bssid: &str) -> ProviderOutcome {
        match self {
            Provider::Apple => apple_lookup(state, bssid).await,
            Provider::Wigle => wigle_lookup(state, bssid).await,
            #[cfg(test)]
            Provider::Mock(m) => m.lookup(bssid).await,
        }
    }
}

/// Test-only mock provider. The closure is invoked per BSSID and decides
/// the outcome; the mock also records every BSSID it was called with so
/// tests can assert call patterns (e.g., to verify in-flight dedup).
#[cfg(test)]
pub struct MockProvider {
    pub name: &'static str,
    respond: Box<dyn Fn(&str) -> ProviderOutcome + Send + Sync>,
    bssids_called: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl MockProvider {
    pub fn new<F>(name: &'static str, respond: F) -> Self
    where
        F: Fn(&str) -> ProviderOutcome + Send + Sync + 'static,
    {
        Self {
            name,
            respond: Box::new(respond),
            bssids_called: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub async fn lookup(&self, bssid: &str) -> ProviderOutcome {
        self.bssids_called
            .lock()
            .expect("mock mutex")
            .push(bssid.to_string());
        (self.respond)(bssid)
    }

    pub fn call_count(&self) -> usize {
        self.bssids_called.lock().expect("mock mutex").len()
    }
}

async fn apple_lookup(state: &Arc<DaemonState>, bssid: &str) -> ProviderOutcome {
    // Apple WPS has no auth and no per-day quota; always callable.
    match state.apple.lookup_bssid(bssid).await {
        Ok(Some(ap)) => ProviderOutcome::Found(ap),
        Ok(None) => ProviderOutcome::NotFound,
        Err(e) => ProviderOutcome::NetworkError(e),
    }
}

/// RAII guard for a reserved WiGLE daily-quota slot (task-0190).
///
/// Armed the moment `try_reserve_api_call` returns true. `defuse()` is called
/// on the two outcomes where a real WiGLE API call actually consumed upstream
/// quota — a `Found` position and an authoritative `NotFound` — so the
/// reservation legitimately stands. Every other exit refunds the slot:
/// `RateLimited` and `Network` (no real unit spent, or rejected upstream —
/// matching prior accounting), AND — the leak this closes — a drop caused by
/// task abort or runtime shutdown while the lookup future is parked at its
/// `.await`. This collapses the two former explicit refund blocks into a
/// single Drop path.
struct QuotaSlot<'a> {
    state: &'a Arc<DaemonState>,
    armed: bool,
}

impl QuotaSlot<'_> {
    /// Mark the reserved slot as legitimately consumed so Drop does not refund
    /// it. Consumes the guard.
    fn defuse(mut self) {
        self.armed = false;
    }
}

impl Drop for QuotaSlot<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // lock_db is never held across an await anywhere in the daemon, so
        // taking it here cannot deadlock; the refund is a single fast SQLite
        // write. Drop must never panic: a refund failure is logged and counted
        // (best-effort), never propagated.
        let refund = {
            let mut db = crate::server::lock_db(self.state);
            db.refund_api_call()
        };
        if let Err(e) = refund {
            warn!("failed to refund WiGLE quota slot on drop: {e}");
            self.state.record_db_failure();
        }
    }
}

async fn wigle_lookup(state: &Arc<DaemonState>, bssid: &str) -> ProviderOutcome {
    // Precondition: credentials configured.
    if !state.wigle.is_configured() {
        return ProviderOutcome::Skipped;
    }

    // Atomically reserve a quota slot BEFORE the network call. This closes the
    // TOCTOU race where two concurrent lookups could both observe count<limit
    // and both proceed, exceeding daily_limit by N.
    let reserved = {
        let mut db = crate::server::lock_db(state);
        match db.try_reserve_api_call(state.args.daily_limit) {
            Ok(b) => b,
            Err(e) => {
                warn!("failed to reserve API slot: {e}");
                return ProviderOutcome::Skipped;
            }
        }
    };
    if !reserved {
        return ProviderOutcome::Skipped;
    }

    // The reservation is now live. This guard refunds it on every exit path —
    // including a cancellation drop at the await below (task abort / process
    // shutdown) — unless defused on an outcome that really consumed upstream
    // WiGLE quota (task-0190).
    let slot = QuotaSlot { state, armed: true };

    match state.wigle.lookup_bssid(bssid).await {
        Ok(ap) => {
            slot.defuse();
            ProviderOutcome::Found(ap)
        }
        Err(WigleError::NotFound) => {
            slot.defuse();
            ProviderOutcome::NotFound
        }
        // RateLimited / Network: no real unit consumed (or rejected upstream).
        // `slot` drops here and refunds, preserving prior accounting.
        Err(WigleError::RateLimited) => ProviderOutcome::HardStop,
        Err(WigleError::Network(e)) => ProviderOutcome::NetworkError(e),
    }
}

#[cfg(test)]
mod tests {
    //! Direct Drop/defuse tests for the WiGLE quota RAII guard (task-0190
    //! AC#4), including the abort-mid-await leak the guard exists to close.
    use super::*;
    use crate::config::Args;
    use crate::db::Database;
    use crate::server::{AddressCache, DaemonState};
    use clap::Parser;

    // NOTE(task-0230): this is the 5th hand-written DaemonState literal (see
    // main/pending/resolver/server). Consolidating them is tracked separately.
    fn test_state() -> Arc<DaemonState> {
        let db = Database::open_memory().unwrap();
        let args = Args::parse_from(["whereamid"]);
        let shutdown = tokio_util::sync::CancellationToken::new();
        Arc::new(DaemonState {
            db: std::sync::Mutex::new(db),
            debouncer: tokio::sync::Mutex::new(crate::debounce::Debouncer::new(10, 5)),
            args,
            wigle: crate::wigle::WigleClient::new("", ""),
            apple: crate::apple::AppleClient::new(),
            nominatim: crate::nominatim::NominatimClient::new(),
            geocode: crate::server::geocode::GeocodeScheduler::new(shutdown.clone()),
            last_fix: tokio::sync::Mutex::new(None),
            inflight: std::sync::Mutex::new(std::collections::HashSet::new()),
            address_cache: std::sync::Mutex::new(AddressCache::new()),
            db_write_failures: std::sync::atomic::AtomicU64::new(0),
            shutdown,
            tasks: std::sync::Mutex::new(tokio::task::JoinSet::new()),
            orphan_tasks_aborted: std::sync::atomic::AtomicU64::new(0),
        })
    }

    fn reserve(state: &Arc<DaemonState>) {
        let mut db = crate::server::lock_db(state);
        assert!(
            db.try_reserve_api_call(state.args.daily_limit).unwrap(),
            "reservation must succeed under the default daily limit"
        );
    }

    fn calls_today(state: &Arc<DaemonState>) -> u32 {
        state.with_db(|db| db.api_calls_today().unwrap())
    }

    /// An armed guard that is simply dropped refunds the reserved slot: the
    /// persisted daily counter returns to its pre-reservation value.
    #[tokio::test]
    async fn quota_slot_drop_refunds_reserved_slot() {
        let state = test_state();
        reserve(&state);
        assert_eq!(calls_today(&state), 1);
        {
            let _slot = QuotaSlot {
                state: &state,
                armed: true,
            };
        } // dropped here -> refund
        assert_eq!(
            calls_today(&state),
            0,
            "dropping an armed guard must refund"
        );
    }

    /// A defused guard leaves the reservation in place: the outcome legitimately
    /// consumed upstream quota (Found / NotFound), so the counter must stay.
    #[tokio::test]
    async fn quota_slot_defuse_keeps_reservation() {
        let state = test_state();
        reserve(&state);
        assert_eq!(calls_today(&state), 1);
        {
            let slot = QuotaSlot {
                state: &state,
                armed: true,
            };
            slot.defuse();
        }
        assert_eq!(calls_today(&state), 1, "defuse must keep the reservation");
    }

    /// The critical case: a task holding an armed guard across an await that
    /// never completes is aborted (as at process shutdown). Drop still runs, so
    /// the reserved slot is refunded rather than leaked into the persisted
    /// counter for the rest of the day.
    #[tokio::test]
    async fn quota_slot_refunds_on_abort_mid_await() {
        let state = test_state();
        reserve(&state);
        assert_eq!(calls_today(&state), 1);

        let task_state = Arc::clone(&state);
        let handle = tokio::spawn(async move {
            let _slot = QuotaSlot {
                state: &task_state,
                armed: true,
            };
            // Park forever; abort drops _slot and triggers the refund.
            std::future::pending::<()>().await;
        });

        // Let the task reach the await, then abort it.
        tokio::task::yield_now().await;
        handle.abort();
        let joined = handle.await;
        assert!(
            joined.unwrap_err().is_cancelled(),
            "task must have been aborted mid-await"
        );

        assert_eq!(
            calls_today(&state),
            0,
            "an aborted mid-await guard must refund the slot, not leak it"
        );
    }
}
