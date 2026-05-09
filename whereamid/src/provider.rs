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
    // Precondition: credentials and daily quota.
    if !state.wigle.is_configured() {
        return ProviderOutcome::Skipped;
    }
    let can_call = {
        let db = crate::server::lock_db(state);
        db.can_call_api(state.args.daily_limit).unwrap_or(false)
    };
    if !can_call {
        return ProviderOutcome::Skipped;
    }

    // NOTE: There is a known TOCTOU race between can_call_api and the
    // record_api_call below — this is tracked as a separate task and is
    // intentionally left unchanged in this refactor.
    match state.wigle.lookup_bssid(bssid).await {
        Ok(ap) => {
            let db = crate::server::lock_db(state);
            if let Err(e) = db.record_api_call() {
                warn!("failed to record API call: {e}");
            }
            ProviderOutcome::Found(ap)
        }
        Err(WigleError::NotFound) => {
            let db = crate::server::lock_db(state);
            if let Err(e) = db.record_api_call() {
                warn!("failed to record API call: {e}");
            }
            ProviderOutcome::NotFound
        }
        Err(WigleError::RateLimited) => ProviderOutcome::HardStop,
        Err(WigleError::Network(e)) => ProviderOutcome::NetworkError(e),
    }
}
