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
    /// A successful HTTP response contained malformed or unexpected JSON.
    ParseError(anyhow::Error),
    /// Network or other transient error.
    Network(anyhow::Error),
}

impl std::fmt::Display for WigleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WigleError::NotFound => write!(f, "BSSID not found in WiGLE"),
            WigleError::RateLimited => write!(f, "WiGLE rate limit exceeded"),
            WigleError::ParseError(e) => write!(f, "WiGLE response parse error: {e}"),
            WigleError::Network(e) => write!(f, "WiGLE network error: {e}"),
        }
    }
}

const WIGLE_SEARCH_URL: &str = "https://api.wigle.net/api/v2/network/search";

pub struct WigleClient {
    client: Client,
    api_user: String,
    api_key: String,
    search_url: String,
}

#[derive(Deserialize, Debug)]
struct WigleSearchResponse {
    success: bool,
    /// Present on error responses; WiGLE reports quota exhaustion as
    /// `{"success":false,"message":"too many queries today"}`. Absent on
    /// success, so default to None.
    #[serde(default)]
    message: Option<String>,
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
            search_url: WIGLE_SEARCH_URL.to_string(),
        }
    }

    /// Test-only constructor for exercising the complete HTTP and provider
    /// path against a deterministic local endpoint. Production constructors
    /// always use the real WiGLE search URL above.
    #[cfg(test)]
    pub(crate) fn with_test_endpoint(
        api_user: &str,
        api_key: &str,
        search_url: String,
        total: std::time::Duration,
    ) -> Self {
        Self {
            client: client_with_timeout(total),
            api_user: api_user.to_string(),
            api_key: api_key.to_string(),
            search_url,
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

        let url = format!("{}?netid={}", self.search_url, bssid);

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

        // Keep response-body transport failures refundable. Only a complete
        // successful response whose bytes do not decode as WiGLE JSON is a
        // ParseError and therefore known to have reached the upstream API.
        let bytes = response
            .bytes()
            .await
            .map_err(|e| WigleError::Network(e.into()))?;
        let body: WigleSearchResponse =
            serde_json::from_slice(&bytes).map_err(|e| WigleError::ParseError(e.into()))?;

        interpret_response(body, bssid)
    }
}

/// Classify a parsed WiGLE search body into an ApInfo or a WigleError.
///
/// Pure (no I/O) so the success / not-found / quota policy is unit-testable
/// without a live HTTP call. WiGLE reports query errors and (historically)
/// daily-quota exhaustion as HTTP 200 with `{"success": false, ...}`; that
/// is NOT an authoritative "not found" and must never be negative-cached.
/// Only `success:true` with zero results is a genuine NotFound (task-0188).
fn interpret_response(body: WigleSearchResponse, bssid: &str) -> Result<ApInfo, WigleError> {
    if !body.success {
        let message = body.message.unwrap_or_default();
        // Quota exhaustion maps to RateLimited so the reserved slot is
        // refunded and the chain hard-stops (see provider.rs), mirroring the
        // HTTP 429 path. Any other success:false is a transient query/server
        // error: still never a NotFound, so the BSSID is not negative-cached
        // and the reserved slot is refunded.
        if looks_like_quota(&message) {
            return Err(WigleError::RateLimited);
        }
        return Err(WigleError::Network(anyhow::anyhow!(
            "WiGLE returned success:false: {message}"
        )));
    }

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

/// Heuristic: does a WiGLE `success:false` message indicate quota/rate
/// exhaustion (vs a query/auth error)? WiGLE reports daily-quota exhaustion
/// as `{"success":false,"message":"too many queries today"}`. Matching is
/// case-insensitive and substring-based; on no match the caller falls back
/// to a transient Network error (still never a negative-cache), so a miss
/// only costs a retry, never a poisoned not_found row.
fn looks_like_quota(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("too many queries") || m.contains("quota") || m.contains("rate limit")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> WigleSearchResponse {
        serde_json::from_str(body).expect("valid WiGLE JSON")
    }

    /// A quota-style success:false maps to RateLimited, which provider.rs
    /// turns into slot-refund + chain HardStop (mirroring HTTP 429). It is
    /// never NotFound. task-0188 AC#2/#3.
    #[test]
    fn success_false_quota_maps_to_rate_limited() {
        let body = parse(r#"{"success":false,"message":"too many queries today"}"#);
        let err = interpret_response(body, "aa:bb:cc:dd:ee:ff").unwrap_err();
        assert!(
            matches!(err, WigleError::RateLimited),
            "quota success:false must be RateLimited, got {err:?}"
        );
    }

    /// A non-quota success:false (query/server error) is a transient Network
    /// error, never NotFound, so the BSSID is not negative-cached.
    /// task-0188 AC#1.
    #[test]
    fn success_false_generic_is_not_not_found() {
        let body = parse(r#"{"success":false,"message":"query failed: malformed netid"}"#);
        let err = interpret_response(body, "aa:bb:cc:dd:ee:ff").unwrap_err();
        assert!(
            !matches!(err, WigleError::NotFound),
            "success:false must never be NotFound, got {err:?}"
        );
        assert!(
            matches!(err, WigleError::Network(_)),
            "non-quota success:false should be transient Network, got {err:?}"
        );
    }

    /// success:false with no message is still never NotFound. task-0188 AC#1.
    #[test]
    fn success_false_no_message_is_not_not_found() {
        let body = parse(r#"{"success":false}"#);
        let err = interpret_response(body, "aa:bb:cc:dd:ee:ff").unwrap_err();
        assert!(
            !matches!(err, WigleError::NotFound),
            "success:false must never be NotFound, got {err:?}"
        );
    }

    /// Control: success:true with zero results is the ONLY genuine NotFound.
    #[test]
    fn success_true_zero_results_is_not_found() {
        let body = parse(r#"{"success":true,"totalResults":0,"results":[]}"#);
        let err = interpret_response(body, "aa:bb:cc:dd:ee:ff").unwrap_err();
        assert!(
            matches!(err, WigleError::NotFound),
            "success:true zero results must be NotFound, got {err:?}"
        );
    }

    /// Control: success:true with a positioned result resolves to ApInfo.
    #[test]
    fn success_true_with_result_is_found() {
        let body = parse(
            r#"{"success":true,"totalResults":1,"results":[{"netid":"AA:BB:CC:DD:EE:FF","ssid":"net","trilat":12.34,"trilong":56.78}]}"#,
        );
        let ap = interpret_response(body, "aa:bb:cc:dd:ee:ff").expect("should resolve");
        assert_eq!(ap.lat, 12.34);
        assert_eq!(ap.lon, 56.78);
        assert!(matches!(ap.source, crate::db::Source::Wigle));
    }
}
