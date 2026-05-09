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
    // network errors, increment the existing pending row's attempts. Don't
    // mark not_found here (drain runs periodically; let the chain
    // succeed eventually or expire via max_attempts).
    let bssids: Vec<String> = pending.iter().map(|p| p.bssid.clone()).collect();
    let policy = ChainPolicy {
        skip_cached: false,
        skip_not_found: false,
        write_through: true,
        delete_pending_on_success: true,
        mark_not_found_per_provider: false,
        mark_not_found_at_chain_end: false,
        on_skipped: SkipAction::NextProvider,
        on_network_error: NetErrorAction::IncrementPending,
        on_hard_stop: HardStopAction::Stop,
    };
    let _ = resolve_chain(&bssids, state, &[Provider::Apple, Provider::Wigle], &policy).await;
}
