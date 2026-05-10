//! Shared HTTP client construction with explicit timeouts.
//!
//! Centralizes timeout policy so no `reqwest::Client::new()` call site
//! ends up with the default (no timeout) — a hung remote would otherwise
//! pin a tokio task and leak memory until process restart.

use std::time::Duration;

use reqwest::Client;

/// Fail fast on TCP connect: 5s.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Default total request timeout for fast endpoints (Apple, WiGLE).
/// 15s covers normal latency plus headroom; longer hangs almost certainly mean
/// a stuck connection rather than slow processing.
pub const REQUEST_TIMEOUT_FAST: Duration = Duration::from_secs(15);

/// Total timeout for Nominatim (OSM public instance, frequently slow).
pub const REQUEST_TIMEOUT_NOMINATIM: Duration = Duration::from_secs(30);

/// Build a reqwest client with the given total request timeout
/// and the shared connect timeout. Falls back to `Client::new()`
/// only if builder fails — which only happens if TLS backend init
/// fails, in which case bigger problems are afoot.
pub fn client_with_timeout(total: Duration) -> Client {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(total)
        .build()
        .expect("reqwest client builder failed (TLS init?)")
}
