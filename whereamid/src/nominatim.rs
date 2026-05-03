//! Reverse geocoding via OpenStreetMap Nominatim.

use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;

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

/// Reverse geocode a lat/lon to an approximate street address.
pub async fn reverse_geocode(lat: f64, lon: f64) -> Result<Address> {
    let client = Client::new();
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
