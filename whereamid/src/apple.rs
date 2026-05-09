//! Apple WPS (Wireless Positioning System) client.
//! Queries Apple's location service for BSSID positions.
//! No authentication required.
//!
//! Protocol: protobuf over HTTPS to gs-loc.apple.com/clls/wloc
//! The protobuf is simple enough to hand-encode/decode without a protobuf library.

use anyhow::{bail, Result};
use reqwest::Client;
use tracing::debug;

use crate::db::ApInfo;
use crate::http::{client_with_timeout, REQUEST_TIMEOUT_FAST};

pub struct AppleClient {
    client: Client,
}

impl Default for AppleClient {
    fn default() -> Self {
        Self {
            client: client_with_timeout(REQUEST_TIMEOUT_FAST),
        }
    }
}

impl AppleClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a single BSSID via Apple WPS. Returns None if not found.
    pub async fn lookup_bssid(&self, bssid: &str) -> Result<Option<ApInfo>> {
        let results = self.lookup_bssids(&[bssid.to_string()]).await?;
        Ok(results.into_iter().next())
    }

    /// Look up multiple BSSIDs in a single request. Returns resolved APs only.
    pub async fn lookup_bssids(&self, bssids: &[String]) -> Result<Vec<ApInfo>> {
        if bssids.is_empty() {
            return Ok(Vec::new());
        }

        debug!("Apple WPS lookup for {} BSSIDs", bssids.len());

        let body = encode_request(bssids);

        let resp = self
            .client
            .post("https://gs-loc.apple.com/clls/wloc")
            .header(
                "User-Agent",
                "locationd/1753.17 CFNetwork/889.9 Darwin/17.2.0",
            )
            .body(body)
            .send()
            .await?;

        if !resp.status().is_success() {
            bail!("Apple WPS returned HTTP {}", resp.status());
        }

        let data = resp.bytes().await?;
        if data.len() < 10 {
            bail!("Apple WPS response too short ({} bytes)", data.len());
        }

        // Skip 10-byte header, parse protobuf
        let results = decode_response(&data[10..], bssids)?;
        Ok(results)
    }
}

/// Encode the Apple WPS protobuf request.
/// Hand-encoded to avoid protobuf dependency.
fn encode_request(bssids: &[String]) -> Vec<u8> {
    // Build the inner protobuf (AppleWLoc message)
    let mut proto = Vec::new();

    // For each BSSID: field 2 (WifiDevice), length-delimited
    for bssid in bssids {
        let mut wifi_device = Vec::new();
        // field 1 (bssid): string
        encode_string(&mut wifi_device, 1, bssid);
        // Wrap as field 2, length-delimited
        encode_bytes(&mut proto, 2, &wifi_device);
    }

    // field 3 (unknown_value1): varint = 0
    encode_varint_field(&mut proto, 3, 0);
    // field 4 (return_single_result): varint = 1
    encode_varint_field(&mut proto, 4, 1);

    // Build the outer envelope
    let mut envelope = Vec::new();
    envelope.extend_from_slice(b"\x00\x01\x00\x05en_US");
    envelope.extend_from_slice(b"\x00\x13com.apple.locationd");
    envelope.extend_from_slice(b"\x00\x0a8.1.12B411");
    envelope.extend_from_slice(b"\x00\x00\x00\x01\x00\x00\x00");
    envelope.push(proto.len() as u8);
    envelope.extend_from_slice(&proto);

    envelope
}

/// Decode the Apple WPS protobuf response.
/// Returns ApInfo for each BSSID that has valid coordinates.
pub fn decode_response(data: &[u8], _requested: &[String]) -> Result<Vec<ApInfo>> {
    let mut results = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        let (field_num, wire_type, new_pos) = decode_tag(data, pos)?;
        pos = new_pos;

        if field_num == 2 && wire_type == 2 {
            // WifiDevice (length-delimited)
            let (msg_data, new_pos) = decode_length_delimited(data, pos)?;
            pos = new_pos;

            if let Some(ap) = parse_wifi_device(msg_data)? {
                results.push(ap);
            }
        } else {
            // Skip unknown field
            pos = skip_field(data, pos, wire_type)?;
        }
    }

    Ok(results)
}

/// Parse a single WifiDevice protobuf message.
pub fn parse_wifi_device(data: &[u8]) -> Result<Option<ApInfo>> {
    let mut bssid: Option<String> = None;
    let mut lat: Option<i64> = None;
    let mut lon: Option<i64> = None;
    let mut pos = 0;

    while pos < data.len() {
        let (field_num, wire_type, new_pos) = decode_tag(data, pos)?;
        pos = new_pos;

        if field_num == 1 && wire_type == 2 {
            // bssid string
            let (s_data, new_pos) = decode_length_delimited(data, pos)?;
            pos = new_pos;
            bssid = Some(String::from_utf8_lossy(s_data).to_string());
        } else if field_num == 2 && wire_type == 2 {
            // Location submessage
            let (loc_data, new_pos) = decode_length_delimited(data, pos)?;
            pos = new_pos;
            let (lt, ln) = parse_location(loc_data)?;
            lat = Some(lt);
            lon = Some(ln);
        } else {
            pos = skip_field(data, pos, wire_type)?;
        }
    }

    match (bssid, lat, lon) {
        (Some(b), Some(lt), Some(ln)) => {
            let lat_f = lt as f64 * 1e-8;
            let lon_f = ln as f64 * 1e-8;
            // Apple returns (-180, -180) for not-found
            if lat_f < -179.0 && lon_f < -179.0 {
                return Ok(None);
            }
            Ok(Some(ApInfo {
                bssid: crate::scanner::normalize_bssid(&b),
                ssid: None,
                lat: lat_f,
                lon: lon_f,
                encryption: None,
                channel: None,
                frequency: None,
                city: None,
                country: None,
                source: "apple".to_string(),
            }))
        }
        _ => Ok(None),
    }
}

/// Parse a Location submessage — fields 1 (lat) and 2 (lon) are varint int64.
/// Apple encodes these as plain signed varints (not zigzag), so we cast directly.
pub fn parse_location(data: &[u8]) -> Result<(i64, i64)> {
    let mut lat: i64 = 0;
    let mut lon: i64 = 0;
    let mut pos = 0;

    while pos < data.len() {
        let (field_num, wire_type, new_pos) = decode_tag(data, pos)?;
        pos = new_pos;

        if wire_type == 0 {
            let (val, new_pos) = decode_varint(data, pos)?;
            pos = new_pos;
            match field_num {
                1 => lat = val as i64,
                2 => lon = val as i64,
                _ => {}
            }
        } else {
            pos = skip_field(data, pos, wire_type)?;
        }
    }

    Ok((lat, lon))
}

// --- Protobuf encoding helpers ---

fn encode_varint(buf: &mut Vec<u8>, mut val: u64) {
    loop {
        let byte = (val & 0x7F) as u8;
        val >>= 7;
        if val == 0 {
            buf.push(byte);
            break;
        }
        buf.push(byte | 0x80);
    }
}

fn encode_varint_field(buf: &mut Vec<u8>, field: u32, val: u64) {
    encode_varint(buf, (field as u64) << 3); // wire type 0 (varint)
    encode_varint(buf, val);
}

fn encode_string(buf: &mut Vec<u8>, field: u32, s: &str) {
    encode_bytes(buf, field, s.as_bytes());
}

fn encode_bytes(buf: &mut Vec<u8>, field: u32, data: &[u8]) {
    encode_varint(buf, ((field as u64) << 3) | 2); // wire type 2
    encode_varint(buf, data.len() as u64);
    buf.extend_from_slice(data);
}

// --- Protobuf decoding helpers ---

fn decode_varint(data: &[u8], mut pos: usize) -> Result<(u64, usize)> {
    let mut val: u64 = 0;
    let mut shift = 0;
    loop {
        if pos >= data.len() {
            bail!("unexpected end of protobuf data");
        }
        let byte = data[pos];
        pos += 1;
        val |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 63 {
            bail!("varint too long");
        }
    }
    Ok((val, pos))
}

fn decode_tag(data: &[u8], pos: usize) -> Result<(u32, u8, usize)> {
    let (val, new_pos) = decode_varint(data, pos)?;
    let field_num = (val >> 3) as u32;
    let wire_type = (val & 0x07) as u8;
    Ok((field_num, wire_type, new_pos))
}

fn decode_length_delimited(data: &[u8], pos: usize) -> Result<(&[u8], usize)> {
    let (len, new_pos) = decode_varint(data, pos)?;
    let len = len as usize;
    let end = new_pos
        .checked_add(len)
        .filter(|&e| e <= data.len())
        .ok_or_else(|| {
            anyhow::anyhow!("protobuf length-delimited field extends past end of data")
        })?;
    Ok((&data[new_pos..end], end))
}

fn skip_field(data: &[u8], pos: usize, wire_type: u8) -> Result<usize> {
    match wire_type {
        0 => {
            // varint
            let (_, new_pos) = decode_varint(data, pos)?;
            Ok(new_pos)
        }
        1 => {
            // 64-bit
            if pos + 8 > data.len() {
                bail!("unexpected end of data for 64-bit field");
            }
            Ok(pos + 8)
        }
        2 => {
            // length-delimited
            let (_, new_pos) = decode_length_delimited(data, pos)?;
            Ok(new_pos)
        }
        5 => {
            // 32-bit
            if pos + 4 > data.len() {
                bail!("unexpected end of data for 32-bit field");
            }
            Ok(pos + 4)
        }
        _ => bail!("unknown wire type {wire_type}"),
    }
}
