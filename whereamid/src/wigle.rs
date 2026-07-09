//! WiGLE API client: lookup a single BSSID.

use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use crate::db::ApInfo;
use crate::http::{client_with_timeout, REQUEST_TIMEOUT_FAST};

/// Error variants specific to WiGLE lookups.
#[derive(Debug)]
pub enum WigleError {
    /// BSSID not found in WiGLE database (HTTP 200, no positioned result).
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
}

#[derive(Deserialize, Debug)]
struct WigleResult {
    // Deserialized from the response's "netid" (the BSSID) but not yet wired
    // through. Kept as a documented placeholder for when a consumer needs it;
    // an explicit `#[allow(dead_code)]` says "parsed-but-currently-unused" far
    // more clearly than a leading-underscore name, which reads as an accidental
    // throwaway a future contributor would delete or fail to wire (task-0138).
    #[allow(dead_code)]
    netid: Option<String>,
    ssid: Option<String>,
    trilat: Option<f64>,
    trilong: Option<f64>,
    encryption: Option<String>,
    // WiGLE also returns a bare `channel`, but it is redundant with `frequency`
    // (band+channel are derivable from it) and band-ambiguous, so it is not
    // parsed: the daemon carries frequency as canonical (task-0123).
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
    ///
    /// PRECONDITION: the caller must have verified `is_configured()`. Production
    /// callers go through the `Provider` abstraction (`provider::wigle_lookup`),
    /// which returns `Skipped` when unconfigured before ever reaching here — so
    /// the precondition is asserted, not handled. A firing assertion means the
    /// Provider abstraction was bypassed (task-0144; previously this was a dead
    /// runtime `WigleError::Network` branch that no caller could reach).
    pub async fn lookup_bssid(&self, bssid: &str) -> std::result::Result<ApInfo, WigleError> {
        debug_assert!(
            self.is_configured(),
            "WigleClient::lookup_bssid called without configured credentials; \
             callers must go through the Provider abstraction"
        );

        debug!("WiGLE lookup for {bssid}");

        // task-0122 AC#10: build the query from a validated Bssid. Parsing here
        // is belt-and-suspenders behind handle_resolve's boundary check
        // (AC#9): a Bssid can only hold six hex-derived octets, so `& # ? /`,
        // whitespace, or `../` can never reach the outbound URL. Reject a
        // malformed value BEFORE the HTTP call rather than interpolating it.
        let canonical = bssid
            .parse::<crate::bssid::Bssid>()
            .map_err(|e| {
                WigleError::Network(anyhow::anyhow!(
                    "refusing to query WiGLE with malformed BSSID {bssid:?}: {e}"
                ))
            })?
            .to_string();

        // `Url::parse_with_params` percent-encodes the value, so even a future
        // caller passing an unexpected string cannot inject extra query
        // parameters — the netid is always a single, encoded parameter.
        let url = reqwest::Url::parse_with_params(&self.search_url, &[("netid", &canonical)])
            .map_err(|e| WigleError::Network(e.into()))?;

        let response = self
            .client
            .get(url)
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
/// Only `success:true` with no result containing a complete coordinate pair is
/// a genuine NotFound (task-0089 and task-0188).
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

    // Use the records themselves as the source of truth. `totalResults` can
    // disagree with the returned array, and a one-sided coordinate is not a
    // usable position. Preserve response order by selecting the first record
    // that contains both coordinates.
    let (r, lat, lon) = body
        .results
        .iter()
        .find_map(|result| match (result.trilat, result.trilong) {
            (Some(lat), Some(lon)) => Some((result, lat, lon)),
            _ => None,
        })
        .ok_or(WigleError::NotFound)?;

    Ok(ApInfo {
        bssid: bssid.to_string(),
        ssid: r.ssid.clone(),
        lat,
        lon,
        encryption: r.encryption.clone(),
        frequency: r.frequency,
        city: r.city.clone(),
        country: r.country.clone(),
        is_mobile: false,
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
    use crate::bssid::Bssid;

    fn parse(body: &str) -> WigleSearchResponse {
        serde_json::from_str(body).expect("valid WiGLE JSON")
    }

    /// task-0122 AC#10: the outbound WiGLE query is built from a validated
    /// Bssid via `Url::parse_with_params`, so it carries exactly one `netid`
    /// parameter and nothing an attacker could smuggle in. The would-be
    /// injecting strings never parse as a Bssid, so they can never reach the
    /// URL builder in the first place.
    #[test]
    fn wigle_query_cannot_be_injection_altered() {
        // A validated BSSID yields a single netid parameter.
        let bssid: Bssid = "AA:BB:CC:DD:EE:FF".parse().unwrap();
        let url =
            reqwest::Url::parse_with_params(WIGLE_SEARCH_URL, &[("netid", &bssid.to_string())])
                .unwrap();
        let params: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(
            params,
            vec![("netid".to_string(), "AA:BB:CC:DD:EE:FF".to_string())],
            "exactly one netid parameter, no injected extras"
        );

        // Injection-shaped strings never become a Bssid, so the lookup path's
        // `bssid.parse::<Bssid>()` rejects them before any URL is built.
        for evil in [
            "aa&x=1",
            "../",
            "AA:BB:CC:DD:EE:FF&netid=evil",
            "aa bb cc dd ee ff",
            "aa:bb:cc:dd:ee:f#",
        ] {
            assert!(
                evil.parse::<Bssid>().is_err(),
                "injection-shaped {evil:?} must fail Bssid parse before URL construction"
            );
        }
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

    /// Control: success:true with no results has no complete coordinate pair.
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

    /// `results` is authoritative even when `totalResults` is inconsistent.
    /// Selection must skip an unpositioned first record and copy all metadata
    /// from the later positioned record, not mix fields across records.
    #[test]
    fn success_true_selects_first_positioned_result_and_its_metadata() {
        let body = parse(
            r#"{
                "success": true,
                "totalResults": 0,
                "results": [
                    {
                        "netid": "AA:BB:CC:DD:EE:FF",
                        "ssid": "unpositioned",
                        "trilat": null,
                        "trilong": null,
                        "encryption": "none",
                        "channel": 1,
                        "frequency": 2412,
                        "city": "Wrong City",
                        "country": "ZZ"
                    },
                    {
                        "netid": "AA:BB:CC:DD:EE:FF",
                        "ssid": "positioned",
                        "trilat": 55.7,
                        "trilong": 12.5,
                        "encryption": "wpa2",
                        "channel": 44,
                        "frequency": 5220,
                        "city": "Copenhagen",
                        "country": "DK"
                    },
                    {
                        "netid": "AA:BB:CC:DD:EE:FF",
                        "ssid": "later-positioned",
                        "trilat": 40.0,
                        "trilong": -74.0,
                        "encryption": "wpa3",
                        "channel": 149,
                        "frequency": 5745,
                        "city": "New York",
                        "country": "US"
                    }
                ]
            }"#,
        );

        let ap = interpret_response(body, "aa:bb:cc:dd:ee:ff").expect("should resolve");

        assert_eq!(ap.bssid, "aa:bb:cc:dd:ee:ff");
        assert_eq!(ap.ssid.as_deref(), Some("positioned"));
        assert_eq!(ap.lat, 55.7);
        assert_eq!(ap.lon, 12.5);
        assert_eq!(ap.encryption.as_deref(), Some("wpa2"));
        assert_eq!(ap.frequency, Some(5220));
        // The band-qualified channel is derived from the canonical frequency
        // (task-0123): 5220 MHz == 5 GHz channel 44.
        assert_eq!(
            ap.frequency
                .and_then(whereami_protocol::Channel::from_frequency),
            Some(whereami_protocol::Channel::FiveGhz(44)),
        );
        assert_eq!(ap.city.as_deref(), Some("Copenhagen"));
        assert_eq!(ap.country.as_deref(), Some("DK"));
        assert!(matches!(ap.source, crate::db::Source::Wigle));
    }

    #[test]
    fn success_true_skips_one_sided_result_for_later_complete_pair() {
        let body = parse(
            r#"{
                "success": true,
                "results": [
                    {"trilat": 1.0, "trilong": null},
                    {"ssid": "complete", "trilat": 55.7, "trilong": 12.5}
                ]
            }"#,
        );

        let ap = interpret_response(body, "aa:bb:cc:dd:ee:ff").expect("should resolve");

        assert_eq!(ap.ssid.as_deref(), Some("complete"));
        assert_eq!((ap.lat, ap.lon), (55.7, 12.5));
    }

    #[test]
    fn success_true_without_any_complete_coordinate_pair_is_not_found() {
        let body = parse(
            r#"{
                "success": true,
                "totalResults": 3,
                "results": [
                    {"trilat": null, "trilong": null},
                    {"trilat": 55.7, "trilong": null},
                    {"trilat": null, "trilong": 12.5}
                ]
            }"#,
        );

        let err = interpret_response(body, "aa:bb:cc:dd:ee:ff").unwrap_err();

        assert!(matches!(err, WigleError::NotFound));
    }
}
