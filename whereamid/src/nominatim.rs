//! Reverse geocoding via OpenStreetMap Nominatim.
//! Rate limit: max 1 request/second per Nominatim usage policy.

use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

use crate::http::{client_with_timeout, REQUEST_TIMEOUT_NOMINATIM};

/// Minimum interval between outbound requests required by Nominatim policy.
/// The scheduler derives its queue-age budget from this single source.
pub(crate) const REQUEST_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Deserialize, Debug)]
struct NominatimResponse {
    display_name: Option<String>,
    address: Option<NominatimAddress>,
}

#[derive(Deserialize, Debug)]
struct NominatimAddress {
    house_number: Option<String>,
    road: Option<String>,
    city: Option<String>,
    postcode: Option<String>,
    country: Option<String>,
}

/// Approximate address from lat/lon via Nominatim.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Address {
    pub display: String,
    pub road: Option<String>,
    pub house_number: Option<String>,
    pub city: Option<String>,
    pub postcode: Option<String>,
    pub country: Option<String>,
}

/// Nominatim reverse geocoding client with rate limiting.
pub struct NominatimClient {
    last_request: Mutex<Instant>,
    transport: Arc<Transport>,
}

type TransportFuture = Pin<Box<dyn Future<Output = Result<Address>> + Send>>;
type Transport = dyn Fn(f64, f64) -> TransportFuture + Send + Sync;

impl NominatimClient {
    /// Default-timeout constructor. Production code uses `with_timeout` to
    /// honour the CLI flag; this exists for tests and external consumers.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::with_timeout(REQUEST_TIMEOUT_NOMINATIM)
    }

    /// Construct with an explicit HTTP request timeout. Used by main.rs to
    /// honour --nominatim-timeout-secs.
    pub fn with_timeout(total: Duration) -> Self {
        let client = client_with_timeout(total);
        Self {
            // Initialize to the past so first request goes through immediately
            last_request: Mutex::new(Instant::now() - Duration::from_secs(2)),
            transport: Arc::new(move |lat, lon| {
                let client = client.clone();
                Box::pin(async move { send_request(&client, lat, lon).await })
            }),
        }
    }

    /// Construct with a deterministic transport for scheduler tests. The
    /// injected operation sits below the real one-second limiter, at exactly
    /// the same boundary as reqwest in production.
    #[cfg(test)]
    pub(crate) fn with_transport<F, Fut>(transport: F) -> Self
    where
        F: Fn(f64, f64) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Address>> + Send + 'static,
    {
        Self {
            last_request: Mutex::new(Instant::now() - Duration::from_secs(2)),
            transport: Arc::new(move |lat, lon| Box::pin(transport(lat, lon))),
        }
    }

    /// Reverse geocode a lat/lon to an approximate street address.
    /// Enforces [`REQUEST_INTERVAL`] between requests per Nominatim policy.
    #[allow(dead_code)]
    pub async fn reverse_geocode(&self, lat: f64, lon: f64) -> Result<Address> {
        self.reverse_geocode_if(lat, lon, || true)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Nominatim request was cancelled before transport"))
    }

    /// Reverse geocode only if `is_current` still holds immediately after the
    /// rate gate and immediately before outbound transport.
    ///
    /// This boundary is intentionally inside the client: checking freshness
    /// before calling `reverse_geocode` is insufficient because the candidate
    /// may be superseded while this method sleeps in the limiter. Returning
    /// `Ok(None)` means no HTTP request was issued and `last_request` was not
    /// advanced, so rejected work never consumes an empty rate slot.
    pub(crate) async fn reverse_geocode_if<F>(
        &self,
        lat: f64,
        lon: f64,
        is_current: F,
    ) -> Result<Option<Address>>
    where
        F: FnOnce() -> bool + Send,
    {
        // Enforce the policy interval since the last outbound request.
        let mut last = self.last_request.lock().await;
        let elapsed = last.elapsed();
        if elapsed < REQUEST_INTERVAL {
            tokio::time::sleep(REQUEST_INTERVAL - elapsed).await;
        }
        // A freshness-rejected candidate issued no request and therefore
        // must not consume a rate slot. Otherwise the valid latest candidate
        // waits another full second and can itself exceed MAX_QUEUE_AGE.
        if !is_current() {
            return Ok(None);
        }
        *last = Instant::now();
        drop(last);

        (self.transport)(lat, lon).await.map(Some)
    }
}

async fn send_request(client: &Client, lat: f64, lon: f64) -> Result<Address> {
    let url = format!(
        "https://nominatim.openstreetmap.org/reverse?lat={lat}&lon={lon}&format=json&addressdetails=1"
    );

    let resp: NominatimResponse = client
        .get(&url)
        .header("User-Agent", "whereami-daemon/0.1")
        .send()
        .await?
        .json()
        .await?;

    // Reject empty / missing display_name (task-0050). Nominatim can
    // return 200 OK with no result for unmapped locations; previously
    // we cached "" as the address, which then leaked back to clients.
    let display = resp
        .display_name
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Nominatim returned empty display_name"))?;

    let addr = resp.address.as_ref();
    Ok(Address {
        display,
        road: addr.and_then(|a| a.road.clone()),
        house_number: addr.and_then(|a| a.house_number.clone()),
        city: addr.and_then(|a| a.city.clone()),
        postcode: addr.and_then(|a| a.postcode.clone()),
        country: addr.and_then(|a| a.country.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// TASK-0193: a candidate rejected after waiting at the real rate gate
    /// issued no transport request, so it must not move `last_request` and
    /// impose a second one-second wait on the valid latest candidate.
    #[tokio::test(start_paused = true)]
    async fn freshness_rejection_does_not_consume_rate_slot() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = NominatimClient::with_transport({
            let calls = Arc::clone(&calls);
            move |_lat, _lon| {
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(Address {
                        display: "test".to_string(),
                        road: None,
                        house_number: None,
                        city: None,
                        postcode: None,
                        country: None,
                    })
                }
            }
        });

        client.reverse_geocode(1.0, 1.0).await.unwrap();
        let rejected = client.reverse_geocode_if(2.0, 2.0, || false).await.unwrap();
        let after_rejection = Instant::now();
        let current = client.reverse_geocode_if(3.0, 3.0, || true).await.unwrap();

        assert!(rejected.is_none());
        assert!(current.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            Instant::now(),
            after_rejection,
            "fresh current work waited on an empty rate slot"
        );
    }
}
