//! Resolver: cache lookup -> provider chain -> pending queue.
//!
//! All Apple/WiGLE cascade variants share a single orchestrator
//! (`resolve_chain`). The three call sites (read-only resolve, background
//! prefetch, pending drain) differ only in policy, not control flow.

use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, warn};

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
    ticket: crate::privacy::PrivacyTicket,
}

impl<'a> InflightGuard<'a> {
    /// Try to claim `bssid`. Returns `Some(guard)` if no other task holds it,
    /// `None` if another task already claimed it.
    fn try_claim(
        state: &'a Arc<DaemonState>,
        bssid: &str,
        ticket: crate::privacy::PrivacyTicket,
    ) -> Option<Self> {
        let mut guard = state.inflight.lock().unwrap_or_else(|e| {
            warn!("inflight mutex was poisoned — recovering");
            e.into_inner()
        });
        if !guard.contains_key(bssid) {
            guard.insert(bssid.to_string(), ticket);
            Some(InflightGuard {
                state,
                bssid: bssid.to_string(),
                ticket,
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
        if guard.get(&self.bssid) == Some(&self.ticket) {
            guard.remove(&self.bssid);
        }
    }
}

/// Result of a resolution operation.
pub struct ResolveResult {
    pub positioned: Vec<ApInfo>,
    /// BSSIDs that came from a remote provider (not the local cache).
    pub fetched_bssids: HashSet<String>,
    /// task-0194: BSSIDs this call established as *authoritatively absent* —
    /// either the not_found table said so within TTL, or the provider chain
    /// reached a definitive absence for it (the task-0189 rule: some provider
    /// authoritatively returned NotFound and nothing was genuinely transient).
    ///
    /// This is the honest, per-call not_found signal `handle_resolve` reports
    /// on the wire. It is deliberately INDEPENDENT of whether the not_found
    /// *table* was written: task-0092 made resolve_readonly classify absence
    /// without persisting it. A BSSID that was merely skipped (missing creds /
    /// exhausted quota), deferred to an in-flight task, or hit a transient
    /// failure is NOT in this set — those are "unresolved", not "not_found".
    pub not_found: HashSet<String>,
}

/// A BSSID paired with the SSID captured by the local scanner before any
/// provider await. `ssid: None` is a real observation (hidden network), not a
/// request to fall back to provider metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedAp {
    pub bssid: String,
    pub ssid: Option<String>,
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
    /// Log and continue. task-0189 removed the former `IncrementPending`
    /// variant: the pending drain now bumps attempts once per pass in
    /// `drain_once` (post-resolve), so per-error increments here would
    /// double-count.
    Ignore,
    /// Insert into the pending queue.
    QueuePending,
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
    /// Mark not_found only when the chain reaches a definitive absence:
    /// not resolved, not deferred, no genuinely-transient outcome, and at
    /// least one provider authoritatively returned NotFound (task-0189).
    /// Used by resolve_background and the pending drain.
    AtChainEnd,
    /// Never write to the not_found table. Used by resolve_readonly
    /// (task-0092): the read-only `resolve` surface must not mutate daemon
    /// state, so a probe that misses does not block the daemon from
    /// re-checking that BSSID for the not_found TTL.
    ///
    /// task-0092 also removed the former `PerProvider` variant (mark on the
    /// first provider NotFound): after resolve_readonly moved to `Never`, no
    /// policy constructed it. Its role — a single-provider chain that marks
    /// immediately — is equivalent to `AtChainEnd` on a one-provider chain,
    /// so nothing was lost.
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
pub async fn lookup_cached(
    bssids: &[String],
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
) -> Vec<ApInfo> {
    let Some(_privacy) = state.privacy.read(ticket).await else {
        return Vec::new();
    };
    match state.db.lookup_cached_touch(bssids.to_vec()).await {
        Ok(result) => result,
        Err(error) => {
            warn!("cached AP lookup failed: {error}");
            Vec::new()
        }
    }
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
    ticket: crate::privacy::PrivacyTicket,
) -> ResolveResult {
    resolve_chain_with_observations(bssids, state, providers, policy, ticket, &[]).await
}

/// Resolve with scanner observations attached to provider results before any
/// write-through cache upsert. This is crate-visible for the pending drain,
/// which reloads the original scanner SSID after a deferred retry.
pub(crate) async fn resolve_chain_with_observations(
    bssids: &[String],
    state: &Arc<DaemonState>,
    providers: &[Provider],
    policy: &ChainPolicy,
    ticket: crate::privacy::PrivacyTicket,
    observations: &[ObservedAp],
) -> ResolveResult {
    let observed_ssids: std::collections::HashMap<&str, &Option<String>> = observations
        .iter()
        .map(|observation| (observation.bssid.as_str(), &observation.ssid))
        .collect();
    resolve_chain_inner(bssids, state, providers, policy, ticket, &observed_ssids).await
}

async fn resolve_chain_inner(
    bssids: &[String],
    state: &Arc<DaemonState>,
    providers: &[Provider],
    policy: &ChainPolicy,
    ticket: crate::privacy::PrivacyTicket,
    observed_ssids: &std::collections::HashMap<&str, &Option<String>>,
) -> ResolveResult {
    let mut positioned: Vec<ApInfo> = Vec::new();
    let mut fetched_bssids: HashSet<String> = HashSet::new();
    let mut resolved: HashSet<String> = HashSet::new();
    let mut to_pend_at_end: Vec<String> = Vec::new();
    // BSSIDs we deferred to a concurrent in-flight task; we must NOT mark
    // them not_found at chain end (the other task owns that decision).
    let mut deferred_to_other: HashSet<String> = HashSet::new();
    // task-0189: the old single `transient_error` set conflated two very
    // different situations. Split into:
    //
    //   hard_transient   — a provider hit a NetworkError or HardStop for this
    //                      BSSID. The miss may be temporary, so we must NOT
    //                      conclude absence; fall back to the pending/attempts
    //                      retry policy (the task-0045 invariant).
    //   authoritative_nf — at least one provider *authoritatively* returned
    //                      NotFound for this BSSID. This is the load-bearing
    //                      positive evidence that grants an end-of-chain mark.
    //
    // A `Skipped` provider (creds unconfigured / daily quota exhausted or
    // corrupt) is a deterministic *abstention*: it is neither transient
    // evidence that blocks a mark nor authoritative evidence that grants one,
    // so it feeds neither set. An all-Skipped BSSID therefore has an empty
    // authoritative_nf and is left pending — never marked. That is what stops
    // credential-free mode from livelocking: before this, Skipped fed transient_error and
    // permanently suppressed every mark, so an Apple-definitive NotFound with
    // WiGLE unconfigured was re-queried on every scan forever.
    //
    // authoritative_nf is also the seed of per-provider not_found granularity
    // (task-0128), which would let us remember "Apple says no" without
    // suppressing a later WiGLE retry.
    let mut hard_transient: HashSet<String> = HashSet::new();
    let mut authoritative_nf: HashSet<String> = HashSet::new();
    // task-0194: BSSIDs authoritatively absent for THIS call — fed by the
    // not_found-table short-circuit below and by the end-of-chain definitive
    // verdict. Returned so handle_resolve can report an honest wire not_found
    // (vs. "unresolved" for skipped/deferred/transient).
    let mut result_not_found: HashSet<String> = HashSet::new();

    // Track providers that have hard-stopped this pass so we don't keep
    // calling them for subsequent BSSIDs.
    let mut stopped: Vec<bool> = vec![false; providers.len()];

    for bssid in bssids {
        // Short-circuit: already in aps cache.
        if policy.skip_cached {
            let cache_hit = match state.privacy.read(ticket).await {
                Some(_privacy) => match state.db.lookup_cached_touch(vec![bssid.clone()]).await {
                    Ok(mut hits) => hits.pop(),
                    Err(e) => {
                        warn!("db error looking up {bssid}: {e}");
                        None
                    }
                },
                None => None,
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
            let is_nf = state
                .db
                .is_not_found(bssid.clone(), state.args.not_found_ttl_days)
                .await
                .unwrap_or(false);
            if is_nf {
                debug!("{bssid} in not_found cache, skipping");
                // task-0194: the not_found table is authoritative within TTL,
                // so this BSSID IS definitively absent for this call.
                result_not_found.insert(bssid.clone());
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
        let _inflight_guard = {
            let Some(_privacy) = state.privacy.read(ticket).await else {
                deferred_to_other.insert(bssid.clone());
                continue;
            };
            match InflightGuard::try_claim(state, bssid, ticket) {
                Some(g) => g,
                None => {
                    debug!("{bssid} already in-flight, deferring to other task");
                    deferred_to_other.insert(bssid.clone());
                    continue;
                }
            }
        };

        // Walk providers in order.
        let mut found = false;
        let mut queue_pending_for_this = false;

        for (idx, provider) in providers.iter().enumerate() {
            if stopped[idx] {
                // A stopped provider is stopped for a transient reason
                // (HardStop is the sole setter of `stopped`), so its
                // absence for THIS bssid is transient too. Without this,
                // an Apple-NotFound after WiGLE hard-stopped earlier in
                // the batch would mark not_found on Apple's word alone —
                // a 30-day blacklist off a momentary 429 shadow.
                hard_transient.insert(bssid.clone());
                continue;
            }

            let outcome = provider.lookup(state, bssid).await;

            match outcome {
                ProviderOutcome::Found(mut ap) => {
                    // Provider SSIDs are not the classification source. Apple
                    // supplies none and WiGLE supplies remote metadata; when a
                    // local scanner observation exists, preserve it exactly —
                    // including None for a hidden network — through the upsert.
                    if let Some(observed_ssid) = observed_ssids.get(bssid.as_str()) {
                        ap.ssid = (*observed_ssid).clone();
                    }
                    // task-0073: per-BSSID resolution at debug! — Apple is
                    // free + unbounded so info! per resolution fills the
                    // journal during fast-scan in busy areas.
                    debug!(
                        "{} resolved {} -> ({}, {})",
                        provider.name(),
                        ap.bssid,
                        ap.lat,
                        ap.lon
                    );
                    if policy.write_through {
                        if let Some(_privacy) = state.privacy.read(ticket).await {
                            if let Err(e) = state
                                .db
                                .cache_resolved_ap(ap.clone(), policy.delete_pending_on_success)
                                .await
                            {
                                warn!("failed to cache AP {} and update pending: {e}", ap.bssid);
                                state.record_db_failure();
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
                    // task-0189: authoritative evidence of absence. Load-bearing
                    // for the end-of-chain mark below. task-0092 removed the
                    // former PerProvider immediate-write branch here (no policy
                    // constructs PerProvider now); the not_found table is
                    // written only at chain end, under AtChainEnd.
                    authoritative_nf.insert(bssid.clone());
                    // Try the next provider in the chain.
                }
                ProviderOutcome::Skipped => {
                    // task-0189: a Skipped provider is a deterministic
                    // abstention (unconfigured creds / exhausted or corrupt
                    // quota), not transient or authoritative evidence. It
                    // feeds neither set; it only drives the per-policy action.
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
                    hard_transient.insert(bssid.clone());
                    match policy.on_hard_stop {
                        HardStopAction::Stop => {}
                        HardStopAction::QueuePendingAndStop => {
                            queue_pending_for_this = true;
                        }
                    }
                }
                ProviderOutcome::NetworkError(e) => {
                    warn!("{} network error for {bssid}: {e}", provider.name());
                    hard_transient.insert(bssid.clone());
                    match policy.on_network_error {
                        NetErrorAction::Ignore => {}
                        NetErrorAction::QueuePending => {
                            queue_pending_for_this = true;
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
        if let Some(_privacy) = state.privacy.read(ticket).await {
            let pending_observations = to_pend_at_end
                .iter()
                .map(|bssid| {
                    (
                        bssid.clone(),
                        observed_ssids
                            .get(bssid.as_str())
                            .map(|ssid| (*ssid).clone())
                            .unwrap_or(None),
                    )
                })
                .collect();
            if let Err(e) = state.db.insert_pending_many(pending_observations).await {
                warn!("failed to insert deferred pending BSSIDs: {e}");
                state.record_db_failure();
            }
        }
    }

    // End-of-chain classification (task-0189 rule, generalised by task-0194).
    // A BSSID is *authoritatively absent for this call* iff ALL of:
    //   - it was not resolved this pass,
    //   - it was not deferred to a concurrent in-flight task,
    //   - no provider hit a genuinely-transient failure (NetworkError /
    //     HardStop) for it, and
    //   - at least one provider *authoritatively* returned NotFound for it.
    //
    // The last conjunct is load-bearing: a BSSID whose providers all merely
    // Skipped had NO provider consulted, so we have no grounds to assert
    // absence — leave it pending rather than invent a not_found. This is what
    // lets credential-free mode (WiGLE unconfigured) record Apple-definitive
    // misses (task-0189 AC#1/AC#4) without livelocking on all-Skipped rows.
    //
    // task-0194: this classification runs for EVERY policy so the returned
    // `not_found` set is honest even for resolve_readonly (Never), which
    // classifies without persisting (task-0092 made table-write and wire-report
    // independent). Only AtChainEnd additionally writes the not_found table and
    // (under delete_pending_on_not_found) drops the pending row. Everything
    // that falls through the `continue`s above is left out of result_not_found
    // and therefore reported as "unresolved" on the wire, not not_found.
    let persist = matches!(policy.not_found, NotFoundPolicy::AtChainEnd);
    for bssid in bssids {
        if resolved.contains(bssid) {
            continue;
        }
        if deferred_to_other.contains(bssid) {
            // The concurrent in-flight task owns the decision (task-0194 AC#2):
            // "unresolved" on the wire, never not_found.
            continue;
        }
        if hard_transient.contains(bssid) {
            // NetworkError / HardStop: the miss may be temporary (task-0045).
            // Fall through to the pending/attempts retry policy: "unresolved".
            continue;
        }
        if !authoritative_nf.contains(bssid) {
            // No provider said NotFound — e.g. every provider Skipped
            // (task-0194 AC#1). Cannot assert absence: "unresolved".
            continue;
        }
        // Definitive absence for this call.
        result_not_found.insert(bssid.clone());
        if persist {
            if let Some(_privacy) = state.privacy.read(ticket).await {
                if let Err(e) = state
                    .db
                    .record_not_found(bssid.clone(), policy.delete_pending_on_not_found)
                    .await
                {
                    warn!("failed to insert not_found {bssid}: {e}");
                    state.record_db_failure();
                }
            }
            // task-0052: collapse drain_cleanup_after_chain — when the chain
            // marks not_found, also remove the pending row so the drain loop
            // doesn't retry it every cycle. Only under delete_pending_on_not_found.
            // `record_not_found` deletes pending in the same transaction when
            // policy requests it.
        }
    }

    ResolveResult {
        positioned,
        fetched_bssids,
        not_found: result_not_found,
    }
}

/// Policy for the read-only `resolve` surface. Extracted (task-0092) so the
/// exact policy resolve_readonly uses is a single source of truth that tests
/// can assert on directly rather than reconstructing.
fn readonly_policy() -> ChainPolicy {
    ChainPolicy {
        skip_cached: true,
        skip_not_found: true,
        write_through: false,
        delete_pending_on_success: false,
        delete_pending_on_not_found: false,
        // task-0092: never write not_found from the read-only surface.
        not_found: NotFoundPolicy::Never,
        on_skipped: SkipAction::NextProvider,
        on_network_error: NetErrorAction::Ignore,
        on_hard_stop: HardStopAction::Stop,
    }
}

/// Resolve BSSIDs for the `resolve` command. Read-only with respect to
/// discovered state (task-0092): it does NOT persist positions to the `aps`
/// cache, does NOT enqueue pending work, and — the task-0092 fix — does NOT
/// write the not_found table.
///
/// The read-only `resolve` surface means "look this up for me, do not change
/// what the daemon believes". The prior policy wrote a WiGLE miss to the
/// not_found table for `not_found_ttl_days` (default 30d), so a user probing a
/// BSSID WiGLE happened not to have would silently block the daemon from
/// re-checking it for a month — a surprise they never asked for. Proactive
/// not_found population is the scan/drain path's job (resolve_background / the
/// pending drain), not this one.
///
/// One benign write remains: a cache HIT bumps that row's `last_seen` via
/// `touch_ap` (shared cache-lookup bookkeeping; the stored position, pending
/// queue, and not_found table are all untouched). Cascade is WiGLE-only,
/// matching prior behaviour.
pub async fn resolve_readonly(
    bssids: &[String],
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
) -> ResolveResult {
    resolve_chain(
        bssids,
        state,
        &[Provider::Wigle],
        &readonly_policy(),
        ticket,
    )
    .await
}

/// Background resolution: resolve uncached stable BSSIDs proactively.
/// Apple first (free), then WiGLE for remaining. Writes to aps cache.
pub async fn resolve_background(
    observations: &[ObservedAp],
    state: &Arc<DaemonState>,
    ticket: crate::privacy::PrivacyTicket,
) {
    let bssids: Vec<String> = observations
        .iter()
        .map(|observation| observation.bssid.clone())
        .collect();
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
    let _ = resolve_chain_with_observations(
        &bssids,
        state,
        &[Provider::Apple, Provider::Wigle],
        &policy,
        ticket,
        observations,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Args;
    use crate::db::{ApInfo, Source};
    use crate::provider::MockProvider;
    use crate::server::{AddressCache, DaemonState};
    use clap::Parser;

    /// Build a minimal DaemonState backed by an in-memory DB.
    /// The Apple/WiGLE/Nominatim clients are real but never called when the
    /// test uses only Provider::Mock(...) as the chain providers.
    fn test_state() -> Arc<DaemonState> {
        let (db_runtime, db, test_db) = crate::db::DbRuntime::open_test().unwrap();
        let args = Args::parse_from(["whereamid"]);
        let shutdown = tokio_util::sync::CancellationToken::new();
        Arc::new(DaemonState {
            db,
            _db_runtime: Some(db_runtime),
            test_db: std::sync::Mutex::new(test_db),
            debouncer: tokio::sync::Mutex::new(crate::debounce::Debouncer::new(10, 5)),
            privacy: crate::privacy::PrivacyCoordinator::new(),
            connection_slots: Arc::new(tokio::sync::Semaphore::new(args.max_connections)),
            args,
            wigle: crate::wigle::WigleClient::new("", ""),
            apple: crate::apple::AppleClient::new(),
            nominatim: crate::nominatim::NominatimClient::new(),
            geocode: crate::server::geocode::GeocodeScheduler::new(shutdown.clone()),
            last_fix: tokio::sync::Mutex::new(None),
            inflight: std::sync::Mutex::new(std::collections::HashMap::new()),
            address_cache: std::sync::Mutex::new(AddressCache::new()),
            db_write_failures: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            api_counter_corruption_denials: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            history_slot: Arc::new(tokio::sync::Semaphore::new(1)),
            wipe_slot: Arc::new(tokio::sync::Semaphore::new(1)),
            shutdown,
            tasks: std::sync::Mutex::new(tokio::task::JoinSet::new()),
            privacy_wipes: std::sync::Mutex::new(tokio::task::JoinSet::new()),
            orphan_tasks_aborted: std::sync::atomic::AtomicU64::new(0),
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
            is_mobile: false,
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
            state.privacy.ticket(),
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
            state.privacy.ticket(),
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
    async fn scanner_ssid_overrides_provider_metadata_for_mobile_classification() {
        let state = test_state();
        let bssids = vec![
            "aa:bb:cc:dd:ee:11".to_owned(),
            "b8:50:01:aa:bb:cc".to_owned(),
            "aa:bb:cc:dd:ee:13".to_owned(),
        ];
        let observations = vec![
            ObservedAp {
                bssid: bssids[0].clone(),
                ssid: Some("MOVIA_BUS_42".to_owned()),
            },
            ObservedAp {
                bssid: bssids[1].clone(),
                ssid: None,
            },
            ObservedAp {
                bssid: bssids[2].clone(),
                ssid: Some("fixed-home-network".to_owned()),
            },
        ];
        let mock = std::sync::Arc::new(MockProvider::new("apple-like", |bssid| {
            let mut ap = mk_ap(bssid, 55.0, 12.0, Source::Apple);
            // Deliberately contradictory provider metadata. The scanner
            // observation must win in all three cases, including explicit
            // hidden/None rather than falling back to this value.
            ap.ssid = match bssid {
                "aa:bb:cc:dd:ee:11" => None,
                _ => Some("DSB_Wi-Fi".to_owned()),
            };
            ProviderOutcome::Found(ap)
        }));

        let result = resolve_chain_with_observations(
            &bssids,
            &state,
            &[Provider::Mock(mock)],
            &permissive_policy(),
            state.privacy.ticket(),
            &observations,
        )
        .await;

        assert_eq!(result.positioned.len(), 3);
        let cached = state.db.lookup_cached_touch(bssids.clone()).await.unwrap();
        assert_eq!(cached.len(), 3);
        assert_eq!(cached[0].ssid.as_deref(), Some("MOVIA_BUS_42"));
        assert!(cached[0].is_mobile, "Apple None must not erase MOVIA");
        assert_eq!(cached[1].ssid, None);
        assert!(
            !cached[1].is_mobile,
            "hidden B8:50:01 must not inherit provider mobile metadata"
        );
        assert_eq!(cached[2].ssid.as_deref(), Some("fixed-home-network"));
        assert!(
            !cached[2].is_mobile,
            "fixed scanner SSID must override provider DSB metadata"
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
            state.privacy.ticket(),
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
            state.privacy.ticket(),
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
        state
            .inflight
            .lock()
            .unwrap()
            .insert(target.clone(), state.privacy.ticket());

        let mock = std::sync::Arc::new(MockProvider::new("blocked", |_| {
            unreachable!("provider must not be called when BSSID is already inflight")
        }));
        let providers = [Provider::Mock(std::sync::Arc::clone(&mock))];

        let result = resolve_chain(
            std::slice::from_ref(&target),
            &state,
            &providers,
            &permissive_policy(),
            state.privacy.ticket(),
        )
        .await;

        assert_eq!(
            mock.call_count(),
            0,
            "deduplicated call must not invoke provider"
        );
        assert!(result.positioned.is_empty());
        // task-0194: a deferred BSSID is NOT authoritative absence — it must
        // not appear in the not_found set (handle_resolve reports it as
        // "unresolved", never "not_found").
        assert!(
            !result.not_found.contains(&target),
            "deferred BSSID must not be classified not_found"
        );

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

    #[tokio::test]
    async fn pre_wipe_inflight_guard_cannot_remove_post_wipe_same_bssid_claim() {
        let state = test_state();
        let bssid = "aa:bb:cc:dd:ee:ab";
        let old_ticket = state.privacy.ticket();
        let old_claim = {
            let _privacy = state.privacy.read(old_ticket).await.unwrap();
            InflightGuard::try_claim(&state, bssid, old_ticket).unwrap()
        };

        let mut wipe = state.privacy.begin_wipe().await;
        wipe.advance_epoch();
        state.inflight.lock().unwrap().clear();
        drop(wipe);

        let new_ticket = state.privacy.ticket();
        let new_claim = {
            let _privacy = state.privacy.read(new_ticket).await.unwrap();
            InflightGuard::try_claim(&state, bssid, new_ticket).unwrap()
        };
        drop(old_claim);
        assert_eq!(
            state.inflight.lock().unwrap().get(bssid),
            Some(&new_ticket),
            "old guard Drop must not erase the post-wipe claim"
        );
        drop(new_claim);
        assert!(!state.inflight.lock().unwrap().contains_key(bssid));
    }

    #[tokio::test]
    async fn stale_resolver_ticket_cannot_repopulate_but_fresh_work_can_resume() {
        let state = test_state();
        let stale = state.privacy.ticket();
        let mut wipe = state.privacy.begin_wipe().await;
        wipe.advance_epoch();
        drop(wipe);

        let target = "aa:bb:cc:dd:ee:ac".to_string();
        let result_bssid = target.clone();
        let provider = std::sync::Arc::new(MockProvider::new("post-wipe", move |_| {
            ProviderOutcome::Found(mk_ap(&result_bssid, 1.0, 2.0, Source::Apple))
        }));
        let result = resolve_chain(
            std::slice::from_ref(&target),
            &state,
            &[Provider::Mock(std::sync::Arc::clone(&provider))],
            &permissive_policy(),
            stale,
        )
        .await;
        assert_eq!(provider.call_count(), 0);
        assert!(result.positioned.is_empty());
        assert_eq!(crate::server::lock_db(&state).cached_ap_count().unwrap(), 0);

        let fresh = state.privacy.ticket();
        let result = resolve_chain(
            std::slice::from_ref(&target),
            &state,
            &[Provider::Mock(std::sync::Arc::clone(&provider))],
            &permissive_policy(),
            fresh,
        )
        .await;
        assert_eq!(provider.call_count(), 1);
        assert_eq!(result.positioned.len(), 1);
        assert_eq!(crate::server::lock_db(&state).cached_ap_count().unwrap(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn provider_result_crossing_wipe_cannot_repopulate_cache() {
        let state = test_state();
        let ticket = state.privacy.ticket();
        let target = "aa:bb:cc:dd:ee:ad".to_string();
        let result_bssid = target.clone();
        let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
        let release = std::sync::Arc::new(std::sync::Barrier::new(2));
        let provider_entered = std::sync::Arc::clone(&entered);
        let provider_release = std::sync::Arc::clone(&release);
        let provider = std::sync::Arc::new(MockProvider::new("blocked", move |_| {
            provider_entered.wait();
            provider_release.wait();
            ProviderOutcome::Found(mk_ap(&result_bssid, 1.0, 2.0, Source::Apple))
        }));

        let resolve_state = Arc::clone(&state);
        let resolve_provider = std::sync::Arc::clone(&provider);
        let resolve_target = target.clone();
        let resolving = tokio::spawn(async move {
            resolve_chain(
                &[resolve_target],
                &resolve_state,
                &[Provider::Mock(resolve_provider)],
                &permissive_policy(),
                ticket,
            )
            .await
        });
        entered.wait();

        // The provider is already on the wire but owns no publication lease.
        // A successful wipe can therefore invalidate it, and the eventual
        // result must fail the cache-write gate.
        let mut wipe = state.privacy.begin_wipe().await;
        wipe.advance_epoch();
        state.inflight.lock().unwrap().clear();
        drop(wipe);
        release.wait();

        let result = resolving.await.unwrap();
        assert_eq!(provider.call_count(), 1);
        assert_eq!(result.positioned.len(), 1, "local stale result may unwind");
        assert_eq!(
            crate::server::lock_db(&state).cached_ap_count().unwrap(),
            0,
            "provider result that crossed the wipe must not publish"
        );
    }

    /// task-0194: ResolveResult.not_found holds ONLY authoritative absences.
    /// A definitive miss (provider NotFound, nothing transient) is included; a
    /// merely-Skipped BSSID (no provider consulted) is not.
    #[tokio::test]
    async fn resolve_chain_not_found_set_is_authoritative_only() {
        let state = test_state();
        let definitive = "aa:bb:cc:dd:ee:d1".to_string();
        let skipped = "aa:bb:cc:dd:ee:d2".to_string();

        let mock = std::sync::Arc::new(MockProvider::new("p", |b| {
            if b.ends_with("d2") {
                ProviderOutcome::Skipped
            } else {
                ProviderOutcome::NotFound
            }
        }));
        let providers = [Provider::Mock(std::sync::Arc::clone(&mock))];

        let result = resolve_chain(
            &[definitive.clone(), skipped.clone()],
            &state,
            &providers,
            &permissive_policy(),
            state.privacy.ticket(),
        )
        .await;

        assert!(
            result.not_found.contains(&definitive),
            "authoritative NotFound must be in the not_found set"
        );
        assert!(
            !result.not_found.contains(&skipped),
            "a merely-Skipped BSSID must NOT be in the not_found set"
        );
    }

    /// task-0092 AC#1: the read-only resolve policy must never write not_found.
    #[test]
    fn resolve_readonly_policy_is_never() {
        assert!(
            matches!(readonly_policy().not_found, NotFoundPolicy::Never),
            "resolve_readonly must use NotFoundPolicy::Never (task-0092)"
        );
    }

    /// task-0092 AC#3: an unknown BSSID probed through the read-only policy —
    /// even when the provider authoritatively returns NotFound — must leave the
    /// not_found table unchanged. Before task-0092 the PerProvider policy wrote
    /// it, blocking re-probes for the 30-day TTL.
    #[tokio::test]
    async fn resolve_readonly_unknown_bssid_does_not_write_not_found() {
        let state = test_state();
        let target = "aa:bb:cc:dd:ee:c1".to_string();

        // A provider that definitively says NotFound — the case the old
        // PerProvider policy would have persisted.
        let mock = std::sync::Arc::new(MockProvider::new("wigle", |_| ProviderOutcome::NotFound));
        let providers = [Provider::Mock(std::sync::Arc::clone(&mock))];

        let result = resolve_chain(
            std::slice::from_ref(&target),
            &state,
            &providers,
            &readonly_policy(),
            state.privacy.ticket(),
        )
        .await;

        assert_eq!(mock.call_count(), 1, "provider must be consulted");
        assert!(
            result.positioned.is_empty(),
            "unknown BSSID has no position"
        );
        assert!(
            !crate::server::lock_db(&state)
                .is_not_found(&target, state.args.not_found_ttl_days)
                .unwrap(),
            "read-only resolve must NOT write the not_found table (task-0092)"
        );
    }

    /// task-0092 AC#4: a cached BSSID is returned from the read-only surface
    /// without inserting/removing an aps row, without touching the pending
    /// queue, and without writing not_found. (The cache-hit path bumps
    /// `last_seen` via touch_ap — benign bookkeeping; the stored position is
    /// unchanged, which is what we assert.) The real WiGLE is never consulted
    /// on a cache hit, so this exercises the actual resolve_readonly function.
    #[tokio::test]
    async fn resolve_readonly_cached_bssid_returns_without_state_writes() {
        let state = test_state();
        let target = "aa:bb:cc:dd:ee:c2".to_string();
        let cached = mk_ap(&target, 55.5, 12.5, Source::Apple);
        crate::server::lock_db(&state).upsert_ap(&cached).unwrap();

        let result = resolve_readonly(
            std::slice::from_ref(&target),
            &state,
            state.privacy.ticket(),
        )
        .await;

        assert_eq!(result.positioned.len(), 1, "cached BSSID must be returned");
        assert!((result.positioned[0].lat - 55.5).abs() < 1e-9);
        assert!((result.positioned[0].lon - 12.5).abs() < 1e-9);
        assert!(
            result.fetched_bssids.is_empty(),
            "a cache hit is not a fetch"
        );

        let db = crate::server::lock_db(&state);
        // Position row still present and unchanged.
        let ap = db.get_ap(&target).unwrap().expect("aps row must survive");
        assert!((ap.lat - 55.5).abs() < 1e-9 && (ap.lon - 12.5).abs() < 1e-9);
        // No not_found write, no pending enqueue.
        assert!(!db
            .is_not_found(&target, state.args.not_found_ttl_days)
            .unwrap());
        assert!(!db.is_pending(&target).unwrap());
    }
}
