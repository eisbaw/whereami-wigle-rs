//! BeaconDB API client: batch geolocation lookup (no auth required).
//! Currently not used in trilateration (it returns aggregate positions, not per-AP),
//! but kept for future use as a fallback when WiGLE is unavailable.
//!
//! The whole module is intentionally unused at runtime; suppress dead-code
//! warnings module-wide rather than annotating every item.
#![allow(dead_code)]

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::http::{client_with_timeout, REQUEST_TIMEOUT_FAST};

/// Result from BeaconDB geolocation.
#[derive(Debug, Clone)]
pub struct BeaconDbResult {
    pub lat: f64,
    pub lon: f64,
    pub accuracy: f64,
}

pub struct BeaconDbClient {
    client: Client,
    enabled: bool,
}

#[derive(Serialize)]
struct GeolocateRequest {
    #[serde(rename = "wifiAccessPoints")]
    wifi_access_points: Vec<WifiAp>,
}

#[derive(Serialize)]
struct WifiAp {
    #[serde(rename = "macAddress")]
    mac_address: String,
}

#[derive(Deserialize, Debug)]
struct GeolocateResponse {
    location: Option<Location>,
    accuracy: Option<f64>,
    fallback: Option<String>,
}

#[derive(Deserialize, Debug)]
struct Location {
    lat: f64,
    lng: f64,
}

impl BeaconDbClient {
    pub fn new(enabled: bool) -> Self {
        Self {
            client: client_with_timeout(REQUEST_TIMEOUT_FAST),
            enabled,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Batch geolocation: send multiple BSSIDs, get a single position estimate.
    /// Returns None if no result or if it fell back to IP geolocation.
    pub async fn geolocate(&self, bssids: &[String]) -> Result<Option<BeaconDbResult>> {
        if !self.enabled || bssids.is_empty() {
            return Ok(None);
        }

        debug!("BeaconDB geolocate for {} BSSIDs", bssids.len());

        let request = GeolocateRequest {
            wifi_access_points: bssids
                .iter()
                .map(|b| WifiAp {
                    mac_address: b.clone(),
                })
                .collect(),
        };

        let response = self
            .client
            .post("https://beacondb.net/v1/geolocate")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            debug!("BeaconDB returned HTTP {}", response.status());
            return Ok(None);
        }

        let body: GeolocateResponse = response.json().await?;

        // Detect IP fallback - treat as not-found
        if body.fallback.as_deref() == Some("ipf") {
            debug!("BeaconDB fell back to IP geolocation, treating as not-found");
            return Ok(None);
        }

        match body.location {
            Some(loc) => Ok(Some(BeaconDbResult {
                lat: loc.lat,
                lon: loc.lng,
                accuracy: body.accuracy.unwrap_or(100.0),
            })),
            None => Ok(None),
        }
    }
}
