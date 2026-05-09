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

use crate::db::ApInfo;
use crate::server::DaemonState;
use crate::wigle::WigleError;

/// Result of a single provider lookup. Application policy (caching, pending
/// queue, not_found marking) is applied by the orchestrator based on this.
pub enum ProviderOutcome {
    /// Provider returned a position for this BSSID.
    Found(ApInfo),
    /// Provider authoritatively says this BSSID is not in its database.
    NotFound,
    /// Provider was not consulted (rate-limited, not configured, etc.).
    /// The chain may try the next provider for the same BSSID.
    Skipped,
    /// A non-recoverable condition for the rest of this chain pass (e.g.,
    /// WiGLE rate-limited). The orchestrator stops calling this provider
    /// for any further BSSIDs in this pass.
    HardStop,
    /// Transient network error. The orchestrator may queue/retry per policy.
    NetworkError(anyhow::Error),
}

/// Static-dispatch enum over the available providers.
pub enum Provider {
    Apple,
    Wigle,
}

impl Provider {
    pub fn name(&self) -> &'static str {
        match self {
            Provider::Apple => "apple",
            Provider::Wigle => "wigle",
        }
    }

    /// Look up a single BSSID via this provider. Performs the provider's
    /// own preconditions and metering. Never touches caches/pending tables.
    pub async fn lookup(&self, state: &Arc<DaemonState>, bssid: &str) -> ProviderOutcome {
        match self {
            Provider::Apple => apple_lookup(state, bssid).await,
            Provider::Wigle => wigle_lookup(state, bssid).await,
        }
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

    // Atomically reserve a quota slot BEFORE the network call. This closes the
    // TOCTOU race where two concurrent lookups could both observe count<limit
    // and both proceed, exceeding daily_limit by N.
    let reserved = {
        let mut db = crate::server::lock_db(state);
        match db.try_reserve_api_call(state.args.daily_limit) {
            Ok(b) => b,
            Err(e) => {
                warn!("failed to reserve API slot: {e}");
                return ProviderOutcome::Skipped;
            }
        }
    };
    if !reserved {
        return ProviderOutcome::Skipped;
    }

    match state.wigle.lookup_bssid(bssid).await {
        Ok(ap) => ProviderOutcome::Found(ap),
        Err(WigleError::NotFound) => ProviderOutcome::NotFound,
        // For RateLimited / Network we did not consume a real WiGLE quota
        // unit (or it was rejected upstream), so refund the local slot to
        // preserve prior accounting behavior.
        Err(WigleError::RateLimited) => {
            let mut db = crate::server::lock_db(state);
            if let Err(e) = db.refund_api_call() {
                warn!("failed to refund API slot after RateLimited: {e}");
            }
            ProviderOutcome::HardStop
        }
        Err(WigleError::Network(e)) => {
            let mut db = crate::server::lock_db(state);
            if let Err(re) = db.refund_api_call() {
                warn!("failed to refund API slot after Network error: {re}");
            }
            ProviderOutcome::NetworkError(e)
        }
    }
}
