//! WiGLE API client: lookup a single BSSID.

use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use crate::db::ApInfo;
use crate::http::{client_with_timeout, REQUEST_TIMEOUT_FAST};

/// Error variants specific to WiGLE lookups.
#[derive(Debug)]
pub enum WigleError {
    /// BSSID not found in WiGLE database (HTTP 200, zero results).
    NotFound,
    /// Rate limited (HTTP 429 or daily quota exceeded).
    RateLimited,
    /// Network or other transient error.
    Network(anyhow::Error),
}

impl std::fmt::Display for WigleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WigleError::NotFound => write!(f, "BSSID not found in WiGLE"),
            WigleError::RateLimited => write!(f, "WiGLE rate limit exceeded"),
            WigleError::Network(e) => write!(f, "WiGLE network error: {e}"),
        }
    }
}

pub struct WigleClient {
    client: Client,
    api_user: String,
    api_key: String,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct WigleSearchResponse {
    success: bool,
    #[serde(default)]
    results: Vec<WigleResult>,
    #[serde(rename = "totalResults")]
    #[serde(default)]
    total_results: i64,
}

#[derive(Deserialize, Debug)]
struct WigleResult {
    #[serde(rename = "netid")]
    _netid: Option<String>,
    ssid: Option<String>,
    trilat: Option<f64>,
    trilong: Option<f64>,
    encryption: Option<String>,
    channel: Option<i32>,
    frequency: Option<i32>,
    city: Option<String>,
    country: Option<String>,
}

impl WigleClient {
    /// Default-timeout constructor. Production code uses `with_timeout` to
    /// honour the CLI flag; this exists for tests and external consumers.
    #[allow(dead_code)]
    pub fn new(api_user: &str, api_key: &str) -> Self {
        Self::with_timeout(api_user, api_key, REQUEST_TIMEOUT_FAST)
    }

    /// Construct with an explicit HTTP request timeout. Used by main.rs to
    /// honour --http-timeout-secs.
    pub fn with_timeout(api_user: &str, api_key: &str, total: std::time::Duration) -> Self {
        Self {
            client: client_with_timeout(total),
            api_user: api_user.to_string(),
            api_key: api_key.to_string(),
        }
    }

    /// Returns true if credentials are configured.
    pub fn is_configured(&self) -> bool {
        !self.api_user.is_empty() && !self.api_key.is_empty()
    }

    /// Look up a BSSID via WiGLE API.
    /// Returns Ok(Some(ApInfo)) on success, Ok(None) should not happen (use WigleError::NotFound).
    pub async fn lookup_bssid(&self, bssid: &str) -> std::result::Result<ApInfo, WigleError> {
        if !self.is_configured() {
            return Err(WigleError::Network(anyhow::anyhow!(
                "WiGLE credentials not configured"
            )));
        }

        debug!("WiGLE lookup for {bssid}");

        let url = format!(
            "https://api.wigle.net/api/v2/network/search?netid={}",
            bssid
        );

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.api_user, Some(&self.api_key))
            .send()
            .await
            .map_err(|e| WigleError::Network(e.into()))?;

        let status = response.status();

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(WigleError::RateLimited);
        }

        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(WigleError::Network(anyhow::anyhow!(
                "WiGLE API returned 403 Forbidden - check credentials"
            )));
        }

        if !status.is_success() {
            return Err(WigleError::Network(anyhow::anyhow!(
                "WiGLE API returned HTTP {status}"
            )));
        }

        let body: WigleSearchResponse = response
            .json()
            .await
            .map_err(|e| WigleError::Network(e.into()))?;

        if body.total_results == 0 || body.results.is_empty() {
            return Err(WigleError::NotFound);
        }

        let r = &body.results[0];
        let lat = r.trilat.ok_or(WigleError::NotFound)?;
        let lon = r.trilong.ok_or(WigleError::NotFound)?;

        Ok(ApInfo {
            bssid: bssid.to_string(),
            ssid: r.ssid.clone(),
            lat,
            lon,
            encryption: r.encryption.clone(),
            channel: r.channel,
            frequency: r.frequency,
            city: r.city.clone(),
            country: r.country.clone(),
            source: crate::db::Source::Wigle,
        })
    }
}
