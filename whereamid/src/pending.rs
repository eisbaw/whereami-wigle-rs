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

    // 1. Try Apple WPS first — one-by-one, no rate limit
    let mut resolved: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in &pending {
        match state.apple.lookup_bssid(&entry.bssid).await {
            Ok(Some(ap)) => {
                info!(
                    "pending drain: Apple resolved {} -> ({}, {})",
                    ap.bssid, ap.lat, ap.lon
                );
                let db = crate::server::lock_db(state);
                if let Err(e) = db.upsert_ap(&ap) {
                    warn!("failed to upsert AP {}: {e}", ap.bssid);
                }
                if let Err(e) = db.delete_pending(&ap.bssid) {
                    warn!("failed to delete pending {}: {e}", ap.bssid);
                }
                resolved.insert(ap.bssid);
            }
            Ok(None) => {
                debug!("pending drain: Apple doesn't know {}", entry.bssid);
            }
            Err(e) => {
                warn!(
                    "pending drain: Apple lookup failed for {}: {e}",
                    entry.bssid
                );
            }
        }
    }

    // 2. WiGLE for remaining unresolved
    for entry in &pending {
        if resolved.contains(&entry.bssid) {
            continue;
        }

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
            debug!("WiGLE not configured, skipping WiGLE drain");
            break;
        }

        match state.wigle.lookup_bssid(&entry.bssid).await {
            Ok(ap) => {
                info!(
                    "pending drain: WiGLE resolved {} -> ({}, {})",
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
                resolved.insert(entry.bssid.clone());
            }
            Err(WigleError::NotFound) => {
                debug!("pending drain: {} not found in WiGLE", entry.bssid);
                let db = crate::server::lock_db(state);
                if let Err(e) = db.record_api_call() {
                    warn!("failed to record API call: {e}");
                }
                // Don't mark not_found yet — only if neither provider found it
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
