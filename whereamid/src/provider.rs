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

async fn wigle_lookup(state: &Arc<DaemonState>, bssid: &str) -> ProviderOutcome {
    // Precondition: credentials configured.
    if !state.wigle.is_configured() {
        return ProviderOutcome::Skipped;
    }

    // Atomically acquire a cancellation-owned quota lease BEFORE the network call. This closes the
    // TOCTOU race where two concurrent lookups could both observe count<limit
    // and both proceed, exceeding daily_limit by N.
    let slot = {
        match state
            .db
            .reserve_api_call(state.args.daily_limit, Arc::clone(&state.db_write_failures))
            .await
        {
            Ok(Some(slot)) => slot,
            Ok(None) => return ProviderOutcome::Skipped,
            Err(e) => {
                warn!("failed to reserve API slot: {e}");
                return ProviderOutcome::Skipped;
            }
        }
    };

    match state.wigle.lookup_bssid(bssid).await {
        Ok(ap) => {
            slot.consume();
            ProviderOutcome::Found(ap)
        }
        Err(WigleError::NotFound) => {
            slot.consume();
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
    //! Direct Drop/consume tests for the WiGLE quota lease (task-0190
    //! AC#4), including the abort-mid-await leak the guard exists to close.
    use super::*;
    use crate::config::Args;
    use crate::server::{AddressCache, DaemonState};
    use clap::Parser;

    // NOTE(task-0230): this is the 5th hand-written DaemonState literal (see
    // main/pending/resolver/server). Consolidating them is tracked separately.
    fn test_state() -> Arc<DaemonState> {
        let (db_runtime, db, test_db) = crate::db::DbRuntime::open_test().unwrap();
        let args = Args::parse_from(["whereamid"]);
        let shutdown = tokio_util::sync::CancellationToken::new();
        Arc::new(DaemonState {
            db,
            _db_runtime: Some(db_runtime),
            test_db: std::sync::Mutex::new(test_db),
            debouncer: tokio::sync::Mutex::new(crate::debounce::Debouncer::new(10, 5)),
            connection_slots: Arc::new(tokio::sync::Semaphore::new(args.max_connections)),
            args,
            wigle: crate::wigle::WigleClient::new("", ""),
            apple: crate::apple::AppleClient::new(),
            nominatim: crate::nominatim::NominatimClient::new(),
            geocode: crate::server::geocode::GeocodeScheduler::new(shutdown.clone()),
            last_fix: tokio::sync::Mutex::new(None),
            inflight: std::sync::Mutex::new(std::collections::HashSet::new()),
            address_cache: std::sync::Mutex::new(AddressCache::new()),
            db_write_failures: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            history_slot: Arc::new(tokio::sync::Semaphore::new(1)),
            shutdown,
            tasks: std::sync::Mutex::new(tokio::task::JoinSet::new()),
            orphan_tasks_aborted: std::sync::atomic::AtomicU64::new(0),
        })
    }

    async fn reserve(state: &Arc<DaemonState>) -> crate::db::QuotaLease {
        state
            .db
            .reserve_api_call(state.args.daily_limit, Arc::clone(&state.db_write_failures))
            .await
            .unwrap()
            .expect("reservation must succeed under the default daily limit")
    }

    async fn calls_today(state: &Arc<DaemonState>) -> u32 {
        state.db.writer_barrier().await.unwrap();
        state.db.api_calls_today().await.unwrap()
    }

    /// An armed guard that is simply dropped refunds the reserved slot: the
    /// persisted daily counter returns to its pre-reservation value.
    #[tokio::test]
    async fn quota_lease_drop_refunds_reserved_slot() {
        let state = test_state();
        let slot = reserve(&state).await;
        assert_eq!(calls_today(&state).await, 1);
        drop(slot);
        assert_eq!(
            calls_today(&state).await,
            0,
            "dropping an armed guard must refund"
        );
    }

    /// A consumed lease leaves the reservation in place: the outcome legitimately
    /// consumed upstream quota (Found / NotFound), so the counter must stay.
    #[tokio::test]
    async fn quota_lease_consume_keeps_reservation() {
        let state = test_state();
        let slot = reserve(&state).await;
        assert_eq!(calls_today(&state).await, 1);
        slot.consume();
        assert_eq!(
            calls_today(&state).await,
            1,
            "consume must keep the reservation"
        );
    }

    /// The critical case: a task holding an active lease across an await that
    /// never completes is aborted (as at process shutdown). Drop still runs, so
    /// the reserved slot is refunded rather than leaked into the persisted
    /// counter for the rest of the day.
    #[tokio::test]
    async fn quota_lease_refunds_on_abort_mid_await() {
        let state = test_state();
        let slot = reserve(&state).await;
        assert_eq!(calls_today(&state).await, 1);

        let (parked_tx, parked_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _slot = slot;
            let _ = parked_tx.send(());
            // Park forever; abort drops _slot and triggers the refund.
            std::future::pending::<()>().await;
        });

        parked_rx
            .await
            .expect("lease holder must reach parked state");
        handle.abort();
        let joined = handle.await;
        assert!(
            joined.unwrap_err().is_cancelled(),
            "task must have been aborted mid-await"
        );

        assert_eq!(
            calls_today(&state).await,
            0,
            "an aborted mid-await guard must refund the slot, not leak it"
        );
    }
}
