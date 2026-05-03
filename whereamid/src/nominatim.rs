//! Reverse geocoding via OpenStreetMap Nominatim.
//! Rate limit: max 1 request/second per Nominatim usage policy.

use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

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
    client: Client,
    last_request: Mutex<Instant>,
}

impl NominatimClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            // Initialize to the past so first request goes through immediately
            last_request: Mutex::new(Instant::now() - Duration::from_secs(2)),
        }
    }

    /// Reverse geocode a lat/lon to an approximate street address.
    /// Enforces 1 request/second rate limit per Nominatim policy.
    pub async fn reverse_geocode(&self, lat: f64, lon: f64) -> Result<Address> {
        // Enforce rate limit: wait until 1s has passed since last request
        {
            let mut last = self.last_request.lock().await;
            let elapsed = last.elapsed();
            if elapsed < Duration::from_secs(1) {
                tokio::time::sleep(Duration::from_secs(1) - elapsed).await;
            }
            *last = Instant::now();
        }

        let url = format!(
            "https://nominatim.openstreetmap.org/reverse?lat={lat}&lon={lon}&format=json&addressdetails=1"
        );

        let resp: NominatimResponse = self
            .client
            .get(&url)
            .header("User-Agent", "whereami-daemon/0.1")
            .send()
            .await?
            .json()
            .await?;

        let addr = resp.address.as_ref();
        Ok(Address {
            display: resp.display_name.unwrap_or_default(),
            road: addr.and_then(|a| a.road.clone()),
            house_number: addr.and_then(|a| a.house_number.clone()),
            city: addr.and_then(|a| a.city.clone()),
            postcode: addr.and_then(|a| a.postcode.clone()),
            country: addr.and_then(|a| a.country.clone()),
        })
    }
}
