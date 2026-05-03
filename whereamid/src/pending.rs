//! Pending queue background drain task.

use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{debug, info, warn};

use crate::server::DaemonState;
use crate::wigle::WigleError;

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

    for entry in &pending {
        // Check rate limit
        let can_call = {
            let db = crate::server::lock_db(state);
            db.can_call_api(state.args.daily_limit).unwrap_or(false)
        };
        if !can_call {
            debug!("daily API limit reached, stopping drain");
            break;
        }

        if !state.wigle.is_configured() {
            debug!("WiGLE not configured, cannot drain pending");
            break;
        }

        match state.wigle.lookup_bssid(&entry.bssid).await {
            Ok(ap) => {
                info!(
                    "pending drain: resolved {} -> ({}, {})",
                    entry.bssid, ap.lat, ap.lon
                );
                let db = crate::server::lock_db(state);
                if let Err(e) = db.record_api_call() {
                    warn!("failed to record API call: {e}");
                }
                if let Err(e) = db.upsert_ap(&ap) {
                    warn!("failed to upsert AP {}: {e}", entry.bssid);
                }
                if let Err(e) = db.delete_pending(&entry.bssid) {
                    warn!("failed to delete pending {}: {e}", entry.bssid);
                }
            }
            Err(WigleError::NotFound) => {
                debug!("pending drain: {} not found in WiGLE", entry.bssid);
                let db = crate::server::lock_db(state);
                if let Err(e) = db.record_api_call() {
                    warn!("failed to record API call: {e}");
                }
                if let Err(e) = db.insert_not_found(&entry.bssid) {
                    warn!("failed to insert not_found {}: {e}", entry.bssid);
                }
                if let Err(e) = db.delete_pending(&entry.bssid) {
                    warn!("failed to delete pending {}: {e}", entry.bssid);
                }
            }
            Err(WigleError::RateLimited) => {
                warn!("pending drain: WiGLE rate limited, stopping");
                break;
            }
            Err(WigleError::Network(e)) => {
                warn!("pending drain: network error for {}: {e}", entry.bssid);
                let db = crate::server::lock_db(state);
                if let Err(e) = db.increment_pending_attempts(&entry.bssid) {
                    warn!("failed to increment attempts for {}: {e}", entry.bssid);
                }
            }
        }
    }
}
