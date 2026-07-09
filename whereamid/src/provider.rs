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

use crate::db::{ApInfo, QuotaReservation};
use crate::server::DaemonState;
use crate::wigle::WigleError;

/// Result of a single provider lookup. Application policy (caching, pending
/// queue, not_found marking) is applied by the orchestrator based on this.
pub enum ProviderOutcome {
    /// Provider returned a position for this BSSID.
    Found(ApInfo),
    /// Provider authoritatively says this BSSID is not in its database.
    NotFound,
    /// Provider was not consulted (quota exhausted/corrupt, not configured).
    /// The chain may try the next provider for the same BSSID.
    Skipped,
    /// A non-recoverable condition for the rest of this chain pass (e.g.,
    /// WiGLE rate-limited or its quota reservation failed at the database).
    /// The orchestrator stops calling this provider for further BSSIDs.
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

    // Atomically acquire a cancellation-owned quota lease BEFORE the network
    // call. This closes the TOCTOU race where concurrent lookups could both
    // observe count<limit and exceed wigle_daily_limit.
    let slot = match reserve_wigle_quota(state).await {
        Ok(slot) => slot,
        Err(outcome) => return outcome,
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
        Err(WigleError::ParseError(error)) => {
            // A complete 2xx response reached us, so WiGLE likely charged the
            // request even though its JSON was malformed. Keep the reserved
            // slot while surfacing a retryable provider failure.
            slot.consume();
            ProviderOutcome::NetworkError(error.context("WiGLE returned an invalid JSON response"))
        }
        // Keep the established RateLimited / Network policy refundable: there
        // was no completed successful response confirming a likely charge.
        // `slot` drops here and refunds prior accounting.
        Err(WigleError::RateLimited) => ProviderOutcome::HardStop,
        Err(WigleError::Network(e)) => ProviderOutcome::NetworkError(e),
    }
}

async fn reserve_wigle_quota(
    state: &Arc<DaemonState>,
) -> std::result::Result<crate::db::QuotaLease, ProviderOutcome> {
    match state
        .db
        .reserve_wigle_lookup(
            state.args.wigle_daily_limit,
            Arc::clone(&state.db_write_failures),
            Arc::clone(&state.api_counter_corruption_denials),
        )
        .await
    {
        Ok(QuotaReservation::Reserved(slot)) => Ok(slot),
        Ok(QuotaReservation::Exhausted | QuotaReservation::CounterCorrupt) => {
            Err(ProviderOutcome::Skipped)
        }
        Err(error) => {
            warn!("failed to reserve WiGLE API slot: {error}");
            state.record_db_failure();
            Err(ProviderOutcome::HardStop)
        }
    }
}

#[cfg(test)]
mod tests {
    //! Direct Drop/consume tests for the WiGLE quota lease (task-0190
    //! AC#4), including the abort-mid-await leak the guard exists to close.
    use super::*;
    use crate::server::DaemonState;

    // task-0230: delegate to the single shared constructor in server.rs rather
    // than hand-writing a DaemonState literal; only the WiGLE client varies here.
    fn test_state() -> Arc<DaemonState> {
        crate::server::test_state()
    }

    fn test_state_with_wigle(api_user: &str, api_key: &str) -> Arc<DaemonState> {
        crate::server::test_state_with_wigle_client(crate::wigle::WigleClient::new(
            api_user, api_key,
        ))
    }

    fn test_state_with_wigle_client(wigle: crate::wigle::WigleClient) -> Arc<DaemonState> {
        crate::server::test_state_with_wigle_client(wigle)
    }

    async fn reserve(state: &Arc<DaemonState>) -> crate::db::QuotaLease {
        match state
            .db
            .reserve_wigle_lookup(
                state.args.wigle_daily_limit,
                Arc::clone(&state.db_write_failures),
                Arc::clone(&state.api_counter_corruption_denials),
            )
            .await
            .unwrap()
        {
            QuotaReservation::Reserved(slot) => slot,
            QuotaReservation::Exhausted | QuotaReservation::CounterCorrupt => {
                panic!("reservation must succeed under the default WiGLE daily limit")
            }
        }
    }

    async fn calls_today(state: &Arc<DaemonState>) -> u32 {
        state.db.writer_barrier().await.unwrap();
        state.db.wigle_lookups_today().await.unwrap()
    }

    async fn malformed_json_server() -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local WiGLE test server");
        let address = listener.local_addr().expect("local test address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept WiGLE request");
            let mut request = [0_u8; 4096];
            let bytes_read = stream.read(&mut request).await.expect("read WiGLE request");
            assert!(
                request[..bytes_read].starts_with(b"GET "),
                "test endpoint must receive an HTTP GET"
            );

            let body = b"this is not JSON";
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(headers.as_bytes())
                .await
                .expect("write response headers");
            stream
                .write_all(body)
                .await
                .expect("write malformed JSON body");
            stream.shutdown().await.expect("close test response");
        });

        (format!("http://{address}/api/v2/network/search"), server)
    }

    // Destination TCP port 0 cannot name a listening service, so this avoids
    // the bind-after-drop race of probing an ephemeral port and releasing it.
    const REFUSED_LOCAL_ENDPOINT: &str = "http://127.0.0.1:0/api/v2/network/search";

    fn seed_api_counter(state: &Arc<DaemonState>, raw: &str) {
        let db = crate::server::lock_db(state);
        db.set_metadata(
            "api_calls_date",
            &chrono::Utc::now().format("%Y-%m-%d").to_string(),
        )
        .unwrap();
        db.set_metadata("wigle_lookups_today", raw).unwrap();
    }

    fn denied_outcome(
        result: std::result::Result<crate::db::QuotaLease, ProviderOutcome>,
    ) -> ProviderOutcome {
        match result {
            Ok(slot) => {
                drop(slot);
                panic!("quota preflight unexpectedly reserved a slot")
            }
            Err(outcome) => outcome,
        }
    }

    #[tokio::test]
    async fn configured_wigle_corrupt_counter_skips_and_counts_without_network() {
        let state = test_state_with_wigle("configured-user", "configured-key");
        assert!(state.wigle.is_configured());
        seed_api_counter(&state, "garbage");

        let outcome = denied_outcome(reserve_wigle_quota(&state).await);
        assert!(matches!(outcome, ProviderOutcome::Skipped));
        assert_eq!(
            state
                .api_counter_corruption_denials
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            state
                .db_write_failures
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            state
                .db
                .get_metadata("wigle_lookups_today")
                .await
                .unwrap()
                .as_deref(),
            Some("garbage")
        );
    }

    #[tokio::test]
    async fn configured_wigle_normal_exhaustion_skips_without_corruption_count() {
        let state = test_state_with_wigle("configured-user", "configured-key");
        assert!(state.wigle.is_configured());
        seed_api_counter(&state, &state.args.wigle_daily_limit.to_string());

        let outcome = denied_outcome(reserve_wigle_quota(&state).await);
        assert!(matches!(outcome, ProviderOutcome::Skipped));
        assert_eq!(
            state
                .api_counter_corruption_denials
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            state
                .db_write_failures
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn configured_wigle_reservation_db_error_hard_stops_without_network() {
        let state = test_state_with_wigle("configured-user", "configured-key");
        assert!(state.wigle.is_configured());
        crate::server::lock_db(&state)
            .drop_metadata_table_for_test()
            .unwrap();

        let outcome = denied_outcome(reserve_wigle_quota(&state).await);
        assert!(matches!(outcome, ProviderOutcome::HardStop));
        assert_eq!(
            state
                .db_write_failures
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            state
                .api_counter_corruption_denials
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn malformed_success_response_consumes_wigle_quota() {
        let (endpoint, server) = malformed_json_server().await;
        let state = test_state_with_wigle_client(crate::wigle::WigleClient::with_test_endpoint(
            "configured-user",
            "configured-key",
            endpoint,
            std::time::Duration::from_secs(2),
        ));
        assert_eq!(calls_today(&state).await, 0);

        let outcome = wigle_lookup(&state, "aa:bb:cc:dd:ee:ff").await;
        let error = match outcome {
            ProviderOutcome::NetworkError(error) => error,
            _ => panic!("malformed 2xx JSON must be a provider network failure"),
        };
        assert!(
            error
                .to_string()
                .contains("WiGLE returned an invalid JSON response"),
            "parse failure context was lost: {error:#}"
        );
        assert_eq!(
            calls_today(&state).await,
            1,
            "WiGLE likely charged a completed 2xx request, so its slot must stay consumed"
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .expect("local WiGLE server must finish")
            .expect("local WiGLE server must not panic");
    }

    #[tokio::test]
    async fn connection_failure_refunds_wigle_quota() {
        let state = test_state_with_wigle_client(crate::wigle::WigleClient::with_test_endpoint(
            "configured-user",
            "configured-key",
            REFUSED_LOCAL_ENDPOINT.to_string(),
            std::time::Duration::from_secs(1),
        ));
        assert_eq!(calls_today(&state).await, 0);

        let outcome = wigle_lookup(&state, "aa:bb:cc:dd:ee:ff").await;
        let error = match outcome {
            ProviderOutcome::NetworkError(error) => error,
            _ => panic!("connection refusal must be surfaced as a network failure"),
        };
        let request_error = error
            .downcast_ref::<reqwest::Error>()
            .expect("network failure must retain the reqwest error");
        assert!(
            request_error.is_connect(),
            "destination port 0 must exercise reqwest's connect failure path: {error:#}"
        );
        assert_eq!(
            calls_today(&state).await,
            0,
            "a request that never connected must refund its reserved WiGLE slot"
        );
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
