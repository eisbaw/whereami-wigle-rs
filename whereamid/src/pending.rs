//! Pending queue background drain task.

use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{debug, info, warn};

use crate::provider::Provider;
use crate::resolver::{resolve_chain, ChainPolicy, HardStopAction, NetErrorAction, SkipAction};
use crate::server::DaemonState;

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
            Err(e) => warn!("failed to delete expired pending: {e}"),
        }
    }

    // Re-check expired not_found entries (30 day TTL)
    {
        let db = crate::server::lock_db(state);
        match db.get_expired_not_found(state.args.not_found_ttl_days, 5) {
            Ok(expired) => {
                for bssid in expired {
                    debug!("re-checking expired not_found entry: {bssid}");
                    if let Err(e) = db.delete_not_found(&bssid) {
                        warn!("failed to delete not_found {bssid}: {e}");
                    }
                    if let Err(e) = db.insert_pending(&bssid, None, None, None, None) {
                        warn!("failed to insert pending {bssid}: {e}");
                    }
                }
            }
            Err(e) => warn!("failed to get expired not_found: {e}"),
        }
    }

    // Pick up to 10 pending MACs
    let pending = {
        let db = crate::server::lock_db(state);
        match db.get_pending(10) {
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
    // mark_not_found_at_chain_end is enabled but the chain itself only
    // fires it when EVERY provider returned a definitive NotFound for the
    // BSSID. Any transient error (network/rate-limit/skipped/hard-stop)
    // suppresses the not_found mark, so a flaky network does not poison
    // the not_found cache. When the mark fires, the chain also leaves the
    // pending row in place; we explicitly remove it here so the BSSID
    // does not retry every drain pass and burn API quota on something we
    // already know nobody can resolve.
    let bssids: Vec<String> = pending.iter().map(|p| p.bssid.clone()).collect();
    let policy = ChainPolicy {
        skip_cached: false,
        skip_not_found: false,
        write_through: true,
        delete_pending_on_success: true,
        mark_not_found_per_provider: false,
        mark_not_found_at_chain_end: true,
        on_skipped: SkipAction::NextProvider,
        on_network_error: NetErrorAction::IncrementPending,
        on_hard_stop: HardStopAction::Stop,
    };
    let result = resolve_chain(&bssids, state, &[Provider::Apple, Provider::Wigle], &policy).await;
    drain_cleanup_after_chain(state, &bssids, &result).await;
}

/// Post-chain cleanup for the drain path: any BSSID the chain just marked
/// not_found in this pass must be removed from pending so it doesn't retry
/// on every drain. Extracted from drain_once so tests can drive the
/// cleanup logic with a Provider::Mock.
async fn drain_cleanup_after_chain(
    state: &Arc<DaemonState>,
    bssids: &[String],
    result: &crate::resolver::ResolveResult,
) {
    // Whatever the chain just marked not_found is now redundantly pending
    // and would retry on every drain. Remove those pending rows so the
    // not_found TTL is the sole gate going forward (task-0045).
    let db = crate::server::lock_db(state);
    for bssid in bssids {
        // resolved == in result.fetched_bssids OR present in result.positioned;
        // but pending-row deletion-on-success was already handled by the chain
        // (delete_pending_on_success=true). We only need to clean up the
        // newly-marked not_found ones here.
        let was_resolved = result.fetched_bssids.contains(bssid)
            || result.positioned.iter().any(|a| &a.bssid == bssid);
        if was_resolved {
            continue;
        }
        // is_not_found() returns true for entries within TTL. If the chain
        // just marked it, this will be true.
        match db.is_not_found(bssid, state.args.not_found_ttl_days) {
            Ok(true) => {
                if let Err(e) = db.delete_pending(bssid) {
                    warn!("failed to delete pending {bssid} after not_found mark: {e}");
                }
            }
            Ok(false) => {}
            Err(e) => warn!("is_not_found probe failed for {bssid}: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Args;
    use crate::db::Database;
    use crate::provider::{MockProvider, ProviderOutcome};
    use crate::resolver::{resolve_chain, ChainPolicy, HardStopAction, NetErrorAction, SkipAction};
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
        })
    }

    fn drain_policy() -> ChainPolicy {
        ChainPolicy {
            skip_cached: false,
            skip_not_found: false,
            write_through: true,
            delete_pending_on_success: true,
            mark_not_found_per_provider: false,
            mark_not_found_at_chain_end: true,
            on_skipped: SkipAction::NextProvider,
            on_network_error: NetErrorAction::IncrementPending,
            on_hard_stop: HardStopAction::Stop,
        }
    }

    /// task-0045 + drain_cleanup_after_chain end-to-end:
    /// definitive NotFound from every provider results in (1) the BSSID
    /// being marked not_found in the DB, AND (2) the BSSID being deleted
    /// from pending so it won't retry on every drain.
    #[tokio::test]
    async fn drain_cleanup_deletes_pending_after_definitive_not_found() {
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
        let result = resolve_chain(
            std::slice::from_ref(&bssid),
            &state,
            &providers,
            &drain_policy(),
        )
        .await;
        drain_cleanup_after_chain(&state, std::slice::from_ref(&bssid), &result).await;

        let nf = crate::server::lock_db(&state)
            .is_not_found(&bssid, state.args.not_found_ttl_days)
            .unwrap();
        assert!(nf, "definitive NotFound must mark not_found");
        let pending = crate::server::lock_db(&state).is_pending(&bssid).unwrap();
        assert!(
            !pending,
            "pending row must be deleted after definitive not_found"
        );
    }

    /// Counterexample: a transient NetworkError must NOT delete the pending
    /// row (the row is still pending, not_found wasn't marked).
    #[tokio::test]
    async fn drain_cleanup_keeps_pending_when_transient_error() {
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
        let result = resolve_chain(
            std::slice::from_ref(&bssid),
            &state,
            &providers,
            &drain_policy(),
        )
        .await;
        drain_cleanup_after_chain(&state, std::slice::from_ref(&bssid), &result).await;

        let nf = crate::server::lock_db(&state)
            .is_not_found(&bssid, state.args.not_found_ttl_days)
            .unwrap();
        assert!(!nf, "transient error must not mark not_found");
        let pending = crate::server::lock_db(&state).is_pending(&bssid).unwrap();
        assert!(pending, "pending row must survive a transient error");
    }
}
