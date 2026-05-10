//! Pending queue background drain task.

use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{debug, info, warn};

use crate::provider::Provider;
use crate::resolver::{
    resolve_chain, ChainPolicy, HardStopAction, NetErrorAction, NotFoundPolicy, SkipAction,
};
use crate::server::DaemonState;

/// Maximum number of expired not_found entries to revive per drain pass.
/// Bounded so a long-tail not_found revival cannot dominate the drain
/// budget; subsequent passes pick up the remaining ones (task-0063).
const NOT_FOUND_REVIVAL_BATCH: usize = 5;
/// Maximum number of pending BSSIDs to attempt to resolve per drain pass.
/// Bounded by --daily-limit and provider rate limits in practice; this
/// is the structural upper bound on per-cycle API spend (task-0063).
const PENDING_DRAIN_BATCH: usize = 10;

/// Spawn the pending queue drain loop. Runs every `pending_interval` seconds.
pub async fn run_pending_drain(state: Arc<DaemonState>) {
    let interval = Duration::from_secs(state.args.pending_interval);
    let max_attempts = state.args.pending_max_attempts;

    info!(
        "pending drain task started (interval: {}s, max_attempts: {})",
        state.args.pending_interval, max_attempts
    );

    loop {
        sleep(interval).await;
        drain_once(&state, max_attempts).await;
    }
}

async fn drain_once(state: &Arc<DaemonState>, max_attempts: i32) {
    // Clean up entries that exceeded max attempts
    {
        let db = crate::server::lock_db(state);
        match db.delete_expired_pending(max_attempts) {
            Ok(deleted) if deleted > 0 => {
                info!("deleted {deleted} pending entries exceeding {max_attempts} attempts");
            }
            Ok(_) => {}
            Err(e) => {
                warn!("failed to delete expired pending: {e}");
                state.record_db_failure();
            }
        }
    }

    // Re-check expired not_found entries (30 day TTL)
    {
        let db = crate::server::lock_db(state);
        match db.get_expired_not_found(state.args.not_found_ttl_days, NOT_FOUND_REVIVAL_BATCH) {
            Ok(expired) => {
                for bssid in expired {
                    debug!("re-checking expired not_found entry: {bssid}");
                    if let Err(e) = db.delete_not_found(&bssid) {
                        warn!("failed to delete not_found {bssid}: {e}");
                        state.record_db_failure();
                    }
                    if let Err(e) = db.insert_pending(&bssid, None, None, None, None) {
                        warn!("failed to insert pending {bssid}: {e}");
                        state.record_db_failure();
                    }
                }
            }
            Err(e) => {
                warn!("failed to get expired not_found: {e}");
                state.record_db_failure();
            }
        }
    }

    // Pick up to PENDING_DRAIN_BATCH pending MACs
    let pending = {
        let db = crate::server::lock_db(state);
        match db.get_pending(PENDING_DRAIN_BATCH) {
            Ok(p) => p,
            Err(e) => {
                warn!("failed to get pending entries: {e}");
                return;
            }
        }
    };

    if pending.is_empty() {
        return;
    }

    debug!("draining {} pending entries", pending.len());

    // The pending list IS the input — they're already known to need
    // resolution, so don't pre-filter on cache or not_found. On WiGLE
    // network errors, increment the existing pending row's attempts.
    //
    // The chain only fires the not_found mark when EVERY provider
    // returned a definitive NotFound for the BSSID (task-0045's
    // transient_error guard). When that happens, delete_pending_on_not_found
    // (task-0052) removes the pending row in the same critical section,
    // so this drain replaces the now-removed drain_cleanup_after_chain.
    let bssids: Vec<String> = pending.iter().map(|p| p.bssid.clone()).collect();
    let policy = ChainPolicy {
        skip_cached: false,
        skip_not_found: false,
        write_through: true,
        delete_pending_on_success: true,
        delete_pending_on_not_found: true,
        not_found: NotFoundPolicy::AtChainEnd,
        on_skipped: SkipAction::NextProvider,
        on_network_error: NetErrorAction::IncrementPending,
        on_hard_stop: HardStopAction::Stop,
    };
    let _ = resolve_chain(&bssids, state, &[Provider::Apple, Provider::Wigle], &policy).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Args;
    use crate::db::Database;
    use crate::provider::{MockProvider, ProviderOutcome};
    use crate::resolver::{
        resolve_chain, ChainPolicy, HardStopAction, NetErrorAction, NotFoundPolicy, SkipAction,
    };
    use crate::server::{AddressCache, DaemonState};
    use clap::Parser;

    fn test_state() -> Arc<DaemonState> {
        let db = Database::open_memory().unwrap();
        let args = Args::parse_from(["whereamid"]);
        Arc::new(DaemonState {
            db: std::sync::Mutex::new(db),
            debouncer: tokio::sync::Mutex::new(crate::debounce::Debouncer::new(10, 5)),
            args,
            wigle: crate::wigle::WigleClient::new("", ""),
            apple: crate::apple::AppleClient::new(),
            nominatim: crate::nominatim::NominatimClient::new(),
            last_fix: tokio::sync::Mutex::new(None),
            inflight: std::sync::Mutex::new(std::collections::HashSet::new()),
            address_cache: std::sync::Mutex::new(AddressCache::new()),
            db_write_failures: std::sync::atomic::AtomicU64::new(0),
        })
    }

    fn drain_policy() -> ChainPolicy {
        ChainPolicy {
            skip_cached: false,
            skip_not_found: false,
            write_through: true,
            delete_pending_on_success: true,
            delete_pending_on_not_found: true,
            not_found: NotFoundPolicy::AtChainEnd,
            on_skipped: SkipAction::NextProvider,
            on_network_error: NetErrorAction::IncrementPending,
            on_hard_stop: HardStopAction::Stop,
        }
    }

    /// task-0045 + task-0052 end-to-end: definitive NotFound from every
    /// provider results in (1) the BSSID being marked not_found in the DB,
    /// AND (2) the BSSID being deleted from pending so it won't retry on
    /// every drain. Both happen inside resolve_chain via the
    /// delete_pending_on_not_found policy bit (drain_cleanup_after_chain
    /// was collapsed in task-0052).
    #[tokio::test]
    async fn drain_chain_marks_not_found_and_deletes_pending_on_definitive_miss() {
        let state = test_state();
        let bssid = "aa:bb:cc:dd:ee:f1".to_string();

        // Seed: BSSID is in pending.
        crate::server::lock_db(&state)
            .insert_pending(&bssid, None, None, None, None)
            .unwrap();
        assert!(crate::server::lock_db(&state).is_pending(&bssid).unwrap());

        let mock_a = std::sync::Arc::new(MockProvider::new("a", |_| ProviderOutcome::NotFound));
        let mock_b = std::sync::Arc::new(MockProvider::new("b", |_| ProviderOutcome::NotFound));
        let providers = [
            crate::provider::Provider::Mock(std::sync::Arc::clone(&mock_a)),
            crate::provider::Provider::Mock(std::sync::Arc::clone(&mock_b)),
        ];
        let _ = resolve_chain(
            std::slice::from_ref(&bssid),
            &state,
            &providers,
            &drain_policy(),
        )
        .await;

        let nf = crate::server::lock_db(&state)
            .is_not_found(&bssid, state.args.not_found_ttl_days)
            .unwrap();
        assert!(nf, "definitive NotFound must mark not_found");
        let pending = crate::server::lock_db(&state).is_pending(&bssid).unwrap();
        assert!(
            !pending,
            "pending row must be deleted by the chain when not_found is marked"
        );
    }

    /// Counterexample: a transient NetworkError must NOT delete the pending
    /// row (the row is still pending, not_found wasn't marked).
    #[tokio::test]
    async fn drain_chain_keeps_pending_when_transient_error() {
        let state = test_state();
        let bssid = "aa:bb:cc:dd:ee:f2".to_string();

        crate::server::lock_db(&state)
            .insert_pending(&bssid, None, None, None, None)
            .unwrap();

        let mock_a = std::sync::Arc::new(MockProvider::new("a", |_| ProviderOutcome::NotFound));
        let mock_b = std::sync::Arc::new(MockProvider::new("b", |_| {
            ProviderOutcome::NetworkError(anyhow::anyhow!("simulated"))
        }));
        let providers = [
            crate::provider::Provider::Mock(std::sync::Arc::clone(&mock_a)),
            crate::provider::Provider::Mock(std::sync::Arc::clone(&mock_b)),
        ];
        let _ = resolve_chain(
            std::slice::from_ref(&bssid),
            &state,
            &providers,
            &drain_policy(),
        )
        .await;

        let nf = crate::server::lock_db(&state)
            .is_not_found(&bssid, state.args.not_found_ttl_days)
            .unwrap();
        assert!(!nf, "transient error must not mark not_found");
        let pending = crate::server::lock_db(&state).is_pending(&bssid).unwrap();
        assert!(pending, "pending row must survive a transient error");
    }
}
