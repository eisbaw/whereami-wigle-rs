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

/// When the chain should record a not_found entry for a missed BSSID.
/// Replaces the prior pair of booleans (mark_not_found_per_provider +
/// mark_not_found_at_chain_end) which permitted two illegal combinations
/// (both true / both false). task-0052.
#[derive(Clone, Copy)]
pub enum NotFoundPolicy {
    /// Mark not_found as soon as any provider returns NotFound. Used by
    /// resolve_readonly which has a single provider.
    PerProvider,
    /// Mark not_found only when every provider in the chain returned a
    /// definitive NotFound. Used by resolve_background and pending drain
    /// (transient errors suppress the mark via task-0045).
    AtChainEnd,
    /// Never mark not_found. No production path uses this today; kept
    /// as a meaningful "off" rather than `Option<NotFoundPolicy>`.
    #[allow(dead_code)]
    Never,
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
    /// On a definitive end-of-chain not_found mark, also delete the
    /// pending row for that BSSID (otherwise the drain loop would retry
    /// every cycle and burn API quota on something every provider just
    /// said doesn't exist). task-0052 collapses drain_cleanup_after_chain.
    pub delete_pending_on_not_found: bool,
    pub not_found: NotFoundPolicy,
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
                    if matches!(policy.not_found, NotFoundPolicy::PerProvider) {
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
    if matches!(policy.not_found, NotFoundPolicy::AtChainEnd) {
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
                //
                // Note: this gate also covers the "we just queued it" case
                // (via on_network_error::QueuePending or on_skipped::
                // QueuePending), since both paths route through
                // transient_error. There is intentionally no is_pending
                // check here — drain_once feeds pending BSSIDs as input
                // and a definitive miss must still mark them not_found.
                continue;
            }
            if let Err(e) = db.insert_not_found(bssid) {
                warn!("failed to insert not_found {bssid}: {e}");
            }
            // task-0052: collapse drain_cleanup_after_chain — when the
            // chain marks not_found, also remove the pending row so the
            // drain loop doesn't retry it every cycle. Only fires under
            // delete_pending_on_not_found.
            if policy.delete_pending_on_not_found {
                if let Err(e) = db.delete_pending(bssid) {
                    warn!("failed to delete pending {bssid} after not_found mark: {e}");
                }
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
        delete_pending_on_not_found: false,
        // Mirror prior behavior: write not_found to avoid burning quota
        // on repeat lookups. PerProvider is fine because the chain has a
        // single provider here.
        not_found: NotFoundPolicy::PerProvider,
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
        delete_pending_on_not_found: false,
        // Don't mark not_found per-provider: Apple may not know it but
        // WiGLE might (and vice versa). Decide at end of chain.
        not_found: NotFoundPolicy::AtChainEnd,
        on_skipped: SkipAction::QueuePending,
        on_network_error: NetErrorAction::QueuePending,
        on_hard_stop: HardStopAction::QueuePendingAndStop,
    };
    let _ = resolve_chain(bssids, state, &[Provider::Apple, Provider::Wigle], &policy).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Args;
    use crate::db::{ApInfo, Database, Source};
    use crate::provider::MockProvider;
    use crate::server::{AddressCache, DaemonState};
    use clap::Parser;

    /// Build a minimal DaemonState backed by an in-memory DB.
    /// The Apple/WiGLE/Nominatim clients are real but never called when the
    /// test uses only Provider::Mock(...) as the chain providers.
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

    fn mk_ap(bssid: &str, lat: f64, lon: f64, source: Source) -> ApInfo {
        ApInfo {
            bssid: bssid.to_string(),
            ssid: None,
            lat,
            lon,
            encryption: None,
            channel: None,
            frequency: None,
            city: None,
            country: None,
            source,
        }
    }

    fn permissive_policy() -> ChainPolicy {
        ChainPolicy {
            skip_cached: true,
            skip_not_found: true,
            write_through: true,
            delete_pending_on_success: false,
            delete_pending_on_not_found: false,
            not_found: NotFoundPolicy::AtChainEnd,
            on_skipped: SkipAction::NextProvider,
            on_network_error: NetErrorAction::Ignore,
            on_hard_stop: HardStopAction::Stop,
        }
    }

    #[tokio::test]
    async fn resolve_chain_short_circuits_on_cache_hit() {
        let state = test_state();
        let cached = mk_ap("aa:bb:cc:dd:ee:01", 55.0, 12.0, Source::Apple);
        crate::server::lock_db(&state).upsert_ap(&cached).unwrap();

        let mock = std::sync::Arc::new(MockProvider::new("mock", |_| {
            ProviderOutcome::Found(mk_ap("zz:zz:zz:zz:zz:zz", 0.0, 0.0, Source::Wigle))
        }));
        let providers = [Provider::Mock(std::sync::Arc::clone(&mock))];

        let result = resolve_chain(
            &["aa:bb:cc:dd:ee:01".to_string()],
            &state,
            &providers,
            &permissive_policy(),
        )
        .await;

        assert_eq!(result.positioned.len(), 1, "cache hit should be returned");
        assert_eq!(
            mock.call_count(),
            0,
            "provider must not be called on cache hit"
        );
        assert!((result.positioned[0].lat - 55.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn resolve_chain_first_provider_found_stops_chain() {
        let state = test_state();
        let target = "aa:bb:cc:dd:ee:02".to_string();

        let mock_a = std::sync::Arc::new(MockProvider::new("a", |bssid| {
            ProviderOutcome::Found(mk_ap(bssid, 1.0, 2.0, Source::Apple))
        }));
        let mock_b = std::sync::Arc::new(MockProvider::new("b", |_| {
            unreachable!("second provider must not be called when first found")
        }));
        let providers = [
            Provider::Mock(std::sync::Arc::clone(&mock_a)),
            Provider::Mock(std::sync::Arc::clone(&mock_b)),
        ];

        let result = resolve_chain(
            std::slice::from_ref(&target),
            &state,
            &providers,
            &permissive_policy(),
        )
        .await;
        assert_eq!(result.positioned.len(), 1);
        assert_eq!(mock_a.call_count(), 1);
        assert_eq!(mock_b.call_count(), 0);
        // Cache write happens because write_through=true.
        let cached = crate::server::lock_db(&state).get_ap(&target).unwrap();
        assert!(
            cached.is_some(),
            "successful resolution must write through to cache"
        );
    }

    #[tokio::test]
    async fn resolve_chain_both_not_found_writes_not_found_table() {
        let state = test_state();
        let target = "aa:bb:cc:dd:ee:03".to_string();

        let mock_a = std::sync::Arc::new(MockProvider::new("a", |_| ProviderOutcome::NotFound));
        let mock_b = std::sync::Arc::new(MockProvider::new("b", |_| ProviderOutcome::NotFound));
        let providers = [
            Provider::Mock(std::sync::Arc::clone(&mock_a)),
            Provider::Mock(std::sync::Arc::clone(&mock_b)),
        ];

        let _ = resolve_chain(
            std::slice::from_ref(&target),
            &state,
            &providers,
            &permissive_policy(),
        )
        .await;

        // Both providers consulted (the chain only stops on Found).
        assert_eq!(mock_a.call_count(), 1);
        assert_eq!(mock_b.call_count(), 1);
        // Definitive miss: not_found is written.
        let is_nf = crate::server::lock_db(&state)
            .is_not_found(&target, state.args.not_found_ttl_days)
            .unwrap();
        assert!(
            is_nf,
            "definitive NotFound from all providers must mark not_found"
        );
    }

    #[tokio::test]
    async fn resolve_chain_transient_error_suppresses_not_found_mark() {
        // task-0045 invariant: when ANY provider returned a transient error
        // (NetworkError / Skipped / HardStop), the chain must NOT mark
        // not_found at chain end — the miss may be temporary.
        let state = test_state();
        let target = "aa:bb:cc:dd:ee:04".to_string();

        let mock_a = std::sync::Arc::new(MockProvider::new("a", |_| ProviderOutcome::NotFound));
        let mock_b = std::sync::Arc::new(MockProvider::new("b", |_| {
            ProviderOutcome::NetworkError(anyhow::anyhow!("simulated"))
        }));
        let providers = [
            Provider::Mock(std::sync::Arc::clone(&mock_a)),
            Provider::Mock(std::sync::Arc::clone(&mock_b)),
        ];

        let _ = resolve_chain(
            std::slice::from_ref(&target),
            &state,
            &providers,
            &permissive_policy(),
        )
        .await;

        let is_nf = crate::server::lock_db(&state)
            .is_not_found(&target, state.args.not_found_ttl_days)
            .unwrap();
        assert!(
            !is_nf,
            "transient NetworkError must suppress not_found mark for {target}"
        );
    }

    #[tokio::test]
    async fn resolve_chain_inflight_dedup_defers_concurrent_lookups() {
        // Two concurrent resolve_chain calls for the same BSSID. The
        // second must observe the BSSID in `state.inflight` and defer —
        // its provider must not be called.
        use tokio::sync::Notify;

        let state = test_state();
        let target = "aa:bb:cc:dd:ee:05".to_string();
        let started = std::sync::Arc::new(Notify::new());
        let release = std::sync::Arc::new(Notify::new());

        // Manually pre-register the BSSID in inflight to simulate another
        // task owning it. This is the simplest deterministic test of the
        // dedup short-circuit (real concurrency is exercised elsewhere via
        // the inflight lock contention paths, but those are racy to assert).
        state.inflight.lock().unwrap().insert(target.clone());

        let mock = std::sync::Arc::new(MockProvider::new("blocked", |_| {
            unreachable!("provider must not be called when BSSID is already inflight")
        }));
        let providers = [Provider::Mock(std::sync::Arc::clone(&mock))];

        let result = resolve_chain(
            std::slice::from_ref(&target),
            &state,
            &providers,
            &permissive_policy(),
        )
        .await;

        assert_eq!(
            mock.call_count(),
            0,
            "deduplicated call must not invoke provider"
        );
        assert!(result.positioned.is_empty());

        // Cleanup the pre-registered inflight entry to keep the suppress
        // check on not_found honest: we deferred to "another task" so the
        // chain must NOT have written not_found for this BSSID.
        state.inflight.lock().unwrap().remove(&target);
        let is_nf = crate::server::lock_db(&state)
            .is_not_found(&target, state.args.not_found_ttl_days)
            .unwrap();
        assert!(
            !is_nf,
            "deferred BSSID must not be marked not_found by this chain"
        );

        // Suppress unused warnings for the Notify imports while keeping
        // the imports available for future lock-based concurrency tests.
        let _ = (started, release);
    }
}
