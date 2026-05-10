//! Resolver: cache lookup -> provider chain -> pending queue.
//!
//! All Apple/WiGLE cascade variants share a single orchestrator
//! (`resolve_chain`). The three call sites (read-only resolve, background
//! prefetch, pending drain) differ only in policy, not control flow.

use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::db::ApInfo;
use crate::provider::{Provider, ProviderOutcome};
use crate::server::DaemonState;

/// RAII guard for an in-flight BSSID claim.
/// Removes the BSSID from `state.inflight` on drop, including unwinding
/// drops, so a panic mid-resolution cannot leak entries and permanently
/// block future resolutions for that BSSID.
struct InflightGuard<'a> {
    state: &'a Arc<DaemonState>,
    bssid: String,
}

impl<'a> InflightGuard<'a> {
    /// Try to claim `bssid`. Returns `Some(guard)` if no other task holds it,
    /// `None` if another task already claimed it.
    fn try_claim(state: &'a Arc<DaemonState>, bssid: &str) -> Option<Self> {
        let mut guard = state.inflight.lock().unwrap_or_else(|e| {
            warn!("inflight mutex was poisoned — recovering");
            e.into_inner()
        });
        if guard.insert(bssid.to_string()) {
            Some(InflightGuard {
                state,
                bssid: bssid.to_string(),
            })
        } else {
            None
        }
    }
}

impl<'a> Drop for InflightGuard<'a> {
    fn drop(&mut self) {
        let mut guard = match self.state.inflight.lock() {
            Ok(g) => g,
            Err(e) => {
                warn!("inflight mutex was poisoned on drop — recovering");
                e.into_inner()
            }
        };
        guard.remove(&self.bssid);
    }
}

/// Result of a resolution operation.
pub struct ResolveResult {
    pub positioned: Vec<ApInfo>,
    /// BSSIDs that came from a remote provider (not the local cache).
    pub fetched_bssids: HashSet<String>,
}

/// What the orchestrator should do when a provider is `Skipped`
/// (rate-limited, not configured, etc.) for a given BSSID.
#[derive(Clone, Copy)]
pub enum SkipAction {
    /// Move on to the next provider for this BSSID.
    NextProvider,
    /// Insert this BSSID into the pending queue (so a later drain retries).
    QueuePending,
}

/// What the orchestrator should do on a transient network error.
#[derive(Clone, Copy)]
pub enum NetErrorAction {
    /// Log and continue.
    Ignore,
    /// Insert into the pending queue.
    QueuePending,
    /// Increment the existing pending row's attempts counter.
    IncrementPending,
}

/// What the orchestrator should do when a provider signals `HardStop`
/// (e.g., WiGLE rate limit hit mid-call). The current provider is
/// abandoned for the rest of this pass either way.
#[derive(Clone, Copy)]
pub enum HardStopAction {
    /// Just stop using this provider.
    Stop,
    /// Stop using this provider, and queue the current BSSID as pending.
    QueuePendingAndStop,
}

/// Cross-cutting policy that distinguishes the three resolution flows.
pub struct ChainPolicy {
    /// Skip BSSIDs already present in the `aps` cache before calling any provider.
    pub skip_cached: bool,
    /// Skip BSSIDs present in the not_found table (within TTL).
    pub skip_not_found: bool,
    /// Persist successful lookups to the `aps` cache (write-through).
    pub write_through: bool,
    /// On a successful resolution, delete the BSSID from the pending queue.
    pub delete_pending_on_success: bool,
    /// On a provider's authoritative NotFound, immediately mark not_found
    /// (instead of waiting for the whole chain to miss).
    pub mark_not_found_per_provider: bool,
    /// At end of chain, mark not_found if no provider resolved it AND it
    /// is not currently in the pending queue.
    pub mark_not_found_at_chain_end: bool,
    pub on_skipped: SkipAction,
    pub on_network_error: NetErrorAction,
    pub on_hard_stop: HardStopAction,
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

/// Generic provider-chain orchestrator.
///
/// For each input BSSID:
///   - optionally short-circuit on cache or not_found,
///   - walk the provider list in order until one returns `Found`,
///   - apply policy (cache write, pending insert, etc.) on each outcome.
///
/// Providers themselves do not touch caches; the chain owns that policy.
pub async fn resolve_chain(
    bssids: &[String],
    state: &Arc<DaemonState>,
    providers: &[Provider],
    policy: &ChainPolicy,
) -> ResolveResult {
    let mut positioned: Vec<ApInfo> = Vec::new();
    let mut fetched_bssids: HashSet<String> = HashSet::new();
    let mut resolved: HashSet<String> = HashSet::new();
    let mut to_pend_at_end: Vec<String> = Vec::new();
    // BSSIDs we deferred to a concurrent in-flight task; we must NOT mark
    // them not_found at chain end (the other task owns that decision).
    let mut deferred_to_other: HashSet<String> = HashSet::new();
    // BSSIDs where at least one provider returned a transient error
    // (network failure, rate limit, hard stop, or skipped). For these we
    // never mark not_found at chain end: the miss may be temporary, so we
    // fall back to the pending/attempts-based retry policy.
    // Only when *every* provider returned a definitive NotFound do we have
    // grounds to write to the not_found table.
    let mut transient_error: HashSet<String> = HashSet::new();

    // Track providers that have hard-stopped this pass so we don't keep
    // calling them for subsequent BSSIDs.
    let mut stopped: Vec<bool> = vec![false; providers.len()];

    for bssid in bssids {
        // Short-circuit: already in aps cache.
        if policy.skip_cached {
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
                        None
                    }
                }
            };
            if let Some(ap) = cache_hit {
                debug!("cache hit for {bssid}");
                positioned.push(ap);
                resolved.insert(bssid.clone());
                continue;
            }
        }

        // Short-circuit: in not_found cache (within TTL).
        if policy.skip_not_found {
            let is_nf = {
                let db = crate::server::lock_db(state);
                db.is_not_found(bssid, state.args.not_found_ttl_days)
                    .unwrap_or(false)
            };
            if is_nf {
                debug!("{bssid} in not_found cache, skipping");
                continue;
            }
        }

        // In-flight dedup: claim this BSSID for the duration of provider work
        // so that a concurrent resolve_chain call does not duplicate
        // Apple/WiGLE traffic. The RAII guard releases on every exit path,
        // including a panic-driven unwind.
        // If another task already owns it, we defer entirely — that task
        // will write to the cache / pending / not_found tables, and a later
        // pass will see the result via the cache.
        let _inflight_guard = match InflightGuard::try_claim(state, bssid) {
            Some(g) => g,
            None => {
                debug!("{bssid} already in-flight, deferring to other task");
                deferred_to_other.insert(bssid.clone());
                continue;
            }
        };

        // Walk providers in order.
        let mut found = false;
        let mut queue_pending_for_this = false;

        for (idx, provider) in providers.iter().enumerate() {
            if stopped[idx] {
                continue;
            }

            let outcome = provider.lookup(state, bssid).await;

            match outcome {
                ProviderOutcome::Found(ap) => {
                    info!(
                        "{} resolved {} -> ({}, {})",
                        provider.name(),
                        ap.bssid,
                        ap.lat,
                        ap.lon
                    );
                    if policy.write_through {
                        let db = crate::server::lock_db(state);
                        if let Err(e) = db.upsert_ap(&ap) {
                            warn!("failed to cache AP {}: {e}", ap.bssid);
                        }
                        if policy.delete_pending_on_success {
                            if let Err(e) = db.delete_pending(&ap.bssid) {
                                warn!("failed to delete pending {}: {e}", ap.bssid);
                            }
                        }
                    }
                    fetched_bssids.insert(bssid.clone());
                    resolved.insert(bssid.clone());
                    positioned.push(ap);
                    found = true;
                    break;
                }
                ProviderOutcome::NotFound => {
                    debug!("{}: {bssid} not found", provider.name());
                    if policy.mark_not_found_per_provider {
                        let db = crate::server::lock_db(state);
                        if let Err(e) = db.insert_not_found(bssid) {
                            warn!("failed to insert not_found {bssid}: {e}");
                        }
                    }
                    // Try the next provider in the chain.
                }
                ProviderOutcome::Skipped => {
                    transient_error.insert(bssid.clone());
                    match policy.on_skipped {
                        SkipAction::NextProvider => {}
                        SkipAction::QueuePending => {
                            queue_pending_for_this = true;
                        }
                    }
                }
                ProviderOutcome::HardStop => {
                    warn!("{} hard-stopped", provider.name());
                    stopped[idx] = true;
                    transient_error.insert(bssid.clone());
                    match policy.on_hard_stop {
                        HardStopAction::Stop => {}
                        HardStopAction::QueuePendingAndStop => {
                            queue_pending_for_this = true;
                        }
                    }
                }
                ProviderOutcome::NetworkError(e) => {
                    warn!("{} network error for {bssid}: {e}", provider.name());
                    transient_error.insert(bssid.clone());
                    match policy.on_network_error {
                        NetErrorAction::Ignore => {}
                        NetErrorAction::QueuePending => {
                            queue_pending_for_this = true;
                        }
                        NetErrorAction::IncrementPending => {
                            let db = crate::server::lock_db(state);
                            if let Err(e2) = db.increment_pending_attempts(bssid) {
                                warn!("failed to increment attempts for {bssid}: {e2}");
                            }
                        }
                    }
                }
            }
        }

        if !found && queue_pending_for_this {
            to_pend_at_end.push(bssid.clone());
        }

        // _inflight_guard drops here, releasing the claim. Pending/not_found
        // inserts after the outer loop are pure DB writes and don't need
        // the BSSID claimed.
    }

    // Apply deferred pending inserts.
    if !to_pend_at_end.is_empty() {
        let db = crate::server::lock_db(state);
        for bssid in &to_pend_at_end {
            if let Err(e) = db.insert_pending(bssid, None, None, None, None) {
                warn!("failed to insert pending {bssid}: {e}");
            }
        }
    }

    // End-of-chain not_found marking: only mark if no provider found it
    // AND it isn't already pending (queued from a network/skipped case).
    //
    // Previously we slurped up to 1000 pending rows into a HashSet to
    // answer membership questions for the (typically few) BSSIDs in this
    // batch. That bounded the cost but silently misclassified pending
    // BSSIDs that fell outside the first 1000 rows. Use the indexed
    // is_pending probe instead — it is O(log n) per BSSID and never lies.
    if policy.mark_not_found_at_chain_end {
        let db = crate::server::lock_db(state);
        for bssid in bssids {
            if resolved.contains(bssid) {
                continue;
            }
            if deferred_to_other.contains(bssid) {
                // The concurrent in-flight task owns the not_found decision
                // for this BSSID; we must not double-write or race it.
                continue;
            }
            if transient_error.contains(bssid) {
                // At least one provider returned a transient error
                // (network/rate-limit/skipped/hard-stop) for this BSSID.
                // The miss may be temporary; fall through to whatever the
                // pending/attempts-based policy decided. This is the
                // "definitive not_found" guard required by task-0045.
                continue;
            }
            match db.is_pending(bssid) {
                Ok(true) => continue,
                Ok(false) => {}
                Err(e) => {
                    warn!("is_pending probe failed for {bssid}: {e}; skipping not_found mark");
                    continue;
                }
            }
            if let Err(e) = db.insert_not_found(bssid) {
                warn!("failed to insert not_found {bssid}: {e}");
            }
        }
    }

    ResolveResult {
        positioned,
        fetched_bssids,
    }
}

/// Resolve BSSIDs for the `resolve` command (ephemeral -- does NOT write to aps cache).
/// Cascade is WiGLE-only: callers expect on-demand resolution against WiGLE,
/// matching prior behavior.
pub async fn resolve_readonly(bssids: &[String], state: &Arc<DaemonState>) -> ResolveResult {
    let policy = ChainPolicy {
        skip_cached: true,
        skip_not_found: true,
        write_through: false,
        delete_pending_on_success: false,
        // Mirror prior behavior: write not_found to avoid burning quota
        // on repeat lookups. Per-provider is fine because the chain has a
        // single provider here.
        mark_not_found_per_provider: true,
        mark_not_found_at_chain_end: false,
        on_skipped: SkipAction::NextProvider,
        on_network_error: NetErrorAction::Ignore,
        on_hard_stop: HardStopAction::Stop,
    };
    resolve_chain(bssids, state, &[Provider::Wigle], &policy).await
}

/// Background resolution: resolve uncached stable BSSIDs proactively.
/// Apple first (free), then WiGLE for remaining. Writes to aps cache.
pub async fn resolve_background(bssids: &[String], state: &Arc<DaemonState>) {
    let policy = ChainPolicy {
        skip_cached: true,
        skip_not_found: true,
        write_through: true,
        delete_pending_on_success: false,
        // Don't mark not_found per-provider: Apple may not know it but
        // WiGLE might (and vice versa). Decide at end of chain.
        mark_not_found_per_provider: false,
        mark_not_found_at_chain_end: true,
        on_skipped: SkipAction::QueuePending,
        on_network_error: NetErrorAction::QueuePending,
        on_hard_stop: HardStopAction::QueuePendingAndStop,
    };
    let _ = resolve_chain(bssids, state, &[Provider::Apple, Provider::Wigle], &policy).await;
}
