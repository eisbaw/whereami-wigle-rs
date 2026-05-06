//! Resolver: cache lookup -> WiGLE API -> BeaconDB fallback -> pending queue.

use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::db::ApInfo;
use crate::server::DaemonState;
use crate::wigle::WigleError;

/// Result of a resolution operation.
pub struct ResolveResult {
    pub positioned: Vec<ApInfo>,
    #[allow(dead_code)]
    pub cached_count: usize,
    #[allow(dead_code)]
    pub fetched_count: usize,
    #[allow(dead_code)]
    pub pending_count: usize,
    pub fetched_bssids: HashSet<String>,
}

/// Resolve BSSIDs synchronously with WiGLE calls. Used by resolve command.
/// Writes to aps cache and pending queue.
#[allow(dead_code)]
pub async fn resolve_for_locate(bssids: &[String], state: &Arc<DaemonState>) -> ResolveResult {
    let mut positioned = Vec::new();
    let mut cached_count = 0;
    let mut fetched_count = 0;
    let mut pending_count = 0;
    let mut fetched_bssids = HashSet::new();
    let mut wigle_exhausted = false;
    let mut uncached_bssids = Vec::new();

    for bssid in bssids {
        // Check aps cache (lock/unlock quickly)
        let cache_hit = {
            let db = crate::server::lock_db(state);
            match db.get_ap(bssid) {
                Ok(Some(ap)) => {
                    if let Err(e) = db.touch_ap(bssid) {
                        warn!("failed to touch AP {bssid}: {e}");
                    }
                    Some(ap)
                }
                Ok(None) => None,
                Err(e) => {
                    warn!("db error looking up {bssid}: {e}");
                    continue;
                }
            }
        };

        if let Some(ap) = cache_hit {
            debug!("cache hit for {bssid}");
            positioned.push(ap);
            cached_count += 1;
            continue;
        }

        // Check not_found cache
        let is_not_found = {
            let db = crate::server::lock_db(state);
            db.is_not_found(bssid, state.args.not_found_ttl_days)
                .unwrap_or(false)
        };
        if is_not_found {
            debug!("{bssid} in not_found cache, skipping");
            continue;
        }

        if wigle_exhausted {
            uncached_bssids.push(bssid.clone());
            continue;
        }

        // Check rate limit
        let can_call = {
            let db = crate::server::lock_db(state);
            db.can_call_api(state.args.daily_limit).unwrap_or(false)
        };
        if !can_call || !state.wigle.is_configured() {
            wigle_exhausted = true;
            uncached_bssids.push(bssid.clone());
            continue;
        }

        // Query WiGLE (async, no lock held)
        match state.wigle.lookup_bssid(bssid).await {
            Ok(ap) => {
                info!("WiGLE resolved {bssid} -> ({}, {})", ap.lat, ap.lon);
                {
                    let db = crate::server::lock_db(state);
                    if let Err(e) = db.record_api_call() {
                        warn!("failed to record API call: {e}");
                    }
                    if let Err(e) = db.upsert_ap(&ap) {
                        warn!("failed to cache AP {bssid}: {e}");
                    }
                }
                fetched_bssids.insert(bssid.clone());
                positioned.push(ap);
                fetched_count += 1;
            }
            Err(WigleError::NotFound) => {
                debug!("WiGLE: {bssid} not found");
                let db = crate::server::lock_db(state);
                if let Err(e) = db.record_api_call() {
                    warn!("failed to record API call: {e}");
                }
                if let Err(e) = db.insert_not_found(bssid) {
                    warn!("failed to insert not_found {bssid}: {e}");
                }
            }
            Err(WigleError::RateLimited) => {
                warn!("WiGLE rate limited");
                wigle_exhausted = true;
                uncached_bssids.push(bssid.clone());
            }
            Err(WigleError::Network(e)) => {
                warn!("WiGLE network error for {bssid}: {e}");
                let db = crate::server::lock_db(state);
                if let Err(e) = db.insert_pending(bssid, None, None, None, None) {
                    warn!("failed to insert pending {bssid}: {e}");
                }
                pending_count += 1;
            }
        }
    }

    // Queue all uncached BSSIDs to pending for later resolution
    if !uncached_bssids.is_empty() {
        // BeaconDB cannot resolve individual AP positions (it returns an aggregate),
        // so we skip it for trilateration and queue everything to pending for WiGLE later.
        let db = crate::server::lock_db(state);
        for bssid in &uncached_bssids {
            if let Err(e) = db.insert_pending(bssid, None, None, None, None) {
                warn!("failed to insert pending {bssid}: {e}");
            }
            pending_count += 1;
        }
    }

    ResolveResult {
        positioned,
        cached_count,
        fetched_count,
        pending_count,
        fetched_bssids,
    }
}

/// Resolve BSSIDs for the `resolve` command (ephemeral -- does NOT write to aps cache).
pub async fn resolve_readonly(bssids: &[String], state: &Arc<DaemonState>) -> ResolveResult {
    let mut positioned = Vec::new();
    let mut cached_count = 0;
    let mut fetched_count = 0;
    let mut fetched_bssids = HashSet::new();

    for bssid in bssids {
        // Check aps cache
        let cache_hit = {
            let db = crate::server::lock_db(state);
            db.get_ap(bssid).ok().flatten()
        };
        if let Some(ap) = cache_hit {
            positioned.push(ap);
            cached_count += 1;
            continue;
        }

        // Check not_found cache
        let is_nf = {
            let db = crate::server::lock_db(state);
            db.is_not_found(bssid, state.args.not_found_ttl_days)
                .unwrap_or(false)
        };
        if is_nf {
            continue;
        }

        // Check rate limit
        let can_call = {
            let db = crate::server::lock_db(state);
            db.can_call_api(state.args.daily_limit).unwrap_or(false)
        };
        if !can_call || !state.wigle.is_configured() {
            continue;
        }

        // Query WiGLE -- ephemeral, do NOT write to aps cache
        match state.wigle.lookup_bssid(bssid).await {
            Ok(ap) => {
                let db = crate::server::lock_db(state);
                if let Err(e) = db.record_api_call() {
                    warn!("failed to record API call: {e}");
                }
                drop(db);
                fetched_bssids.insert(bssid.clone());
                positioned.push(ap);
                fetched_count += 1;
            }
            Err(WigleError::NotFound) => {
                let db = crate::server::lock_db(state);
                if let Err(e) = db.record_api_call() {
                    warn!("failed to record API call: {e}");
                }
                // Write to not_found to avoid burning quota on repeated lookups
                if let Err(e) = db.insert_not_found(bssid) {
                    warn!("failed to insert not_found {bssid}: {e}");
                }
            }
            Err(WigleError::RateLimited) => {
                warn!("WiGLE rate limited during resolve");
                break;
            }
            Err(WigleError::Network(e)) => {
                warn!("WiGLE network error for {bssid}: {e}");
            }
        }
    }

    ResolveResult {
        positioned,
        cached_count,
        fetched_count,
        pending_count: 0,
        fetched_bssids,
    }
}

/// Cache-only lookup: returns positions from the aps table, no API calls.
/// Used by `locate` to give instant responses.
pub fn lookup_cached(bssids: &[String], state: &Arc<DaemonState>) -> Vec<ApInfo> {
    let db = crate::server::lock_db(state);
    let mut result = Vec::new();
    for bssid in bssids {
        match db.get_ap(bssid) {
            Ok(Some(ap)) => {
                if let Err(e) = db.touch_ap(bssid) {
                    warn!("failed to touch AP {bssid}: {e}");
                }
                result.push(ap);
            }
            Ok(None) => {}
            Err(e) => warn!("db error looking up {bssid}: {e}"),
        }
    }
    result
}

/// Background resolution: resolve uncached stable BSSIDs proactively.
/// Queries all providers (Apple first as batch, then WiGLE for remaining).
/// Takes the union of results. Writes to aps cache on success.
pub async fn resolve_background(bssids: &[String], state: &Arc<DaemonState>) {
    // Filter to only uncached, unknown BSSIDs
    let to_resolve: Vec<String> = bssids
        .iter()
        .filter(|bssid| {
            let db = crate::server::lock_db(state);
            let cached = db.get_ap(bssid).ok().flatten().is_some();
            let not_found = db
                .is_not_found(bssid, state.args.not_found_ttl_days)
                .unwrap_or(false);
            !cached && !not_found
        })
        .cloned()
        .collect();

    if to_resolve.is_empty() {
        return;
    }

    let mut resolved: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 1. Apple WPS — batch lookup, no auth, no rate limit
    match state.apple.lookup_bssids(&to_resolve).await {
        Ok(aps) => {
            let db = crate::server::lock_db(state);
            for ap in aps {
                info!("Apple resolved {} -> ({}, {})", ap.bssid, ap.lat, ap.lon);
                if let Err(e) = db.upsert_ap(&ap) {
                    warn!("failed to cache AP {}: {e}", ap.bssid);
                }
                resolved.insert(ap.bssid);
            }
        }
        Err(e) => {
            warn!("Apple WPS batch lookup failed: {e}");
        }
    }

    // 2. WiGLE — one-by-one for remaining unresolved BSSIDs
    for bssid in &to_resolve {
        if resolved.contains(bssid) {
            continue;
        }

        // Check rate limit
        let can_call = {
            let db = crate::server::lock_db(state);
            db.can_call_api(state.args.daily_limit).unwrap_or(false)
        };
        if !can_call || !state.wigle.is_configured() {
            let db = crate::server::lock_db(state);
            if let Err(e) = db.insert_pending(bssid, None, None, None, None) {
                warn!("failed to insert pending {bssid}: {e}");
            }
            continue;
        }

        match state.wigle.lookup_bssid(bssid).await {
            Ok(ap) => {
                info!("WiGLE resolved {} -> ({}, {})", bssid, ap.lat, ap.lon);
                let db = crate::server::lock_db(state);
                if let Err(e) = db.record_api_call() {
                    warn!("failed to record API call: {e}");
                }
                if let Err(e) = db.upsert_ap(&ap) {
                    warn!("failed to cache AP {bssid}: {e}");
                }
                resolved.insert(bssid.clone());
            }
            Err(WigleError::NotFound) => {
                debug!("WiGLE: {bssid} not found");
                let db = crate::server::lock_db(state);
                if let Err(e) = db.record_api_call() {
                    warn!("failed to record API call: {e}");
                }
                // Don't insert not_found — Apple might know it even if WiGLE doesn't
            }
            Err(WigleError::RateLimited) => {
                warn!("WiGLE rate limited, stopping");
                let db = crate::server::lock_db(state);
                if let Err(e) = db.insert_pending(bssid, None, None, None, None) {
                    warn!("failed to insert pending {bssid}: {e}");
                }
                break;
            }
            Err(WigleError::Network(e)) => {
                warn!("WiGLE network error for {bssid}: {e}");
                let db = crate::server::lock_db(state);
                if let Err(e) = db.insert_pending(bssid, None, None, None, None) {
                    warn!("failed to insert pending {bssid}: {e}");
                }
            }
        }
    }

    // BSSIDs not resolved by any provider — only mark not_found if neither found them
    for bssid in &to_resolve {
        if resolved.contains(bssid) {
            continue;
        }
        // Check if it's already pending (from WiGLE errors above)
        let db = crate::server::lock_db(state);
        let in_pending = db
            .get_pending(1000)
            .unwrap_or_default()
            .iter()
            .any(|p| p.bssid == *bssid);
        if !in_pending {
            // Neither provider found it and no network error — genuinely unknown
            if let Err(e) = db.insert_not_found(bssid) {
                warn!("failed to insert not_found {bssid}: {e}");
            }
        }
    }
}
