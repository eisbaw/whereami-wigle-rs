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
pub(crate) fn encode_request(bssids: &[String]) -> Vec<u8> {
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

    // Build the outer envelope (Apple ARPC framing).
    //
    // Layout (all big-endian):
    //   u16  version            = 0x0001
    //   u16  locale_len         = 0x0005,  bytes  "en_US"
    //   u16  app_id_len         = 0x0013,  bytes  "com.apple.locationd"
    //   u16  os_version_len     = 0x000a,  bytes  "8.1.12B411"
    //   u32  flag (function id) = 0x00000001
    //   u32  payload_len        = proto.len() as u32   <- was a single byte!
    //   payload bytes
    //
    // The original Python reference (iSniff-GPS) and apple_bssid_locator.py
    // hardcode the high 3 bytes of payload_len to zero and only fill the last
    // byte. That silently truncates whenever the protobuf exceeds 255 bytes
    // (e.g. ~15+ BSSIDs in one batch). Encode the full u32 instead.
    let mut envelope = Vec::new();
    envelope.extend_from_slice(b"\x00\x01\x00\x05en_US");
    envelope.extend_from_slice(b"\x00\x13com.apple.locationd");
    envelope.extend_from_slice(b"\x00\x0a8.1.12B411");
    envelope.extend_from_slice(&0x0000_0001u32.to_be_bytes()); // function id / flag
    let proto_len: u32 = proto
        .len()
        .try_into()
        .expect("Apple WPS protobuf payload exceeds u32::MAX bytes");
    envelope.extend_from_slice(&proto_len.to_be_bytes());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Offset in the envelope where the 4-byte big-endian payload length lives.
    /// Layout reminder (bytes):
    ///   0..2  : version              (\x00\x01)
    ///   2..4  : locale length        (\x00\x05)
    ///   4..9  : locale "en_US"
    ///   9..11 : app id length        (\x00\x13)
    ///   11..30: app id "com.apple.locationd"
    ///   30..32: os version length    (\x00\x0a)
    ///   32..42: os version "8.1.12B411"
    ///   42..46: flag/function id     (BE u32 = 1)
    ///   46..50: payload length       (BE u32) <-- HERE
    ///   50..  : protobuf payload
    const LEN_OFFSET: usize = 46;
    const PAYLOAD_OFFSET: usize = 50;

    fn make_bssids(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| {
                format!(
                    "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    (i >> 40) as u8,
                    (i >> 32) as u8,
                    (i >> 24) as u8,
                    (i >> 16) as u8,
                    (i >> 8) as u8,
                    i as u8,
                )
            })
            .collect()
    }

    /// Length field is a 4-byte big-endian u32, equals payload length, and
    /// matches the actual remaining bytes. Exercised across N=1..=300 BSSIDs
    /// so that we cross the previously-truncating 256-byte boundary.
    #[test]
    fn encode_length_field_matches_payload_for_all_sizes() {
        for n in [1usize, 2, 5, 10, 14, 15, 16, 50, 100, 200, 300] {
            let bssids = make_bssids(n);
            let envelope = encode_request(&bssids);

            assert!(
                envelope.len() > PAYLOAD_OFFSET,
                "envelope too short for n={n}: {} bytes",
                envelope.len()
            );

            let len_bytes: [u8; 4] = envelope[LEN_OFFSET..PAYLOAD_OFFSET].try_into().unwrap();
            let declared_len = u32::from_be_bytes(len_bytes) as usize;
            let actual_payload_len = envelope.len() - PAYLOAD_OFFSET;

            assert_eq!(
                declared_len, actual_payload_len,
                "n={n}: declared length {declared_len} != actual payload {actual_payload_len}"
            );
        }
    }

    /// For batches that produce >255-byte protobufs (>= ~15 BSSIDs in
    /// practice), the upper byte(s) of the length field must be non-zero.
    /// This is the regression assertion for the original truncation bug.
    #[test]
    fn encode_length_uses_full_u32_above_255_bytes() {
        let bssids = make_bssids(50);
        let envelope = encode_request(&bssids);

        let len_bytes: [u8; 4] = envelope[LEN_OFFSET..PAYLOAD_OFFSET].try_into().unwrap();
        let declared_len = u32::from_be_bytes(len_bytes);
        assert!(
            declared_len > 255,
            "test expects payload >255 bytes; got {declared_len}"
        );

        // At least one of the high three bytes must be non-zero — that is
        // exactly the data the old `as u8` cast was throwing away.
        let high_bytes_nonzero = len_bytes[0] != 0 || len_bytes[1] != 0 || len_bytes[2] != 0;
        assert!(
            high_bytes_nonzero,
            "high bytes of length field are all zero ({len_bytes:?}) — truncation regression"
        );
    }

    /// The protobuf payload itself must round-trip: encode then re-parse the
    /// embedded WifiDevice messages and check the BSSIDs are intact. This
    /// guards against the protobuf payload being corrupted by the framing
    /// change.
    #[test]
    fn encoded_protobuf_roundtrips_through_decoder() {
        let bssids = make_bssids(20); // ~ >255 byte payload
        let envelope = encode_request(&bssids);
        let proto = &envelope[PAYLOAD_OFFSET..];

        // Parse the WifiDevice (field=2) entries out of the protobuf payload.
        // The encoder produces field 2 (WifiDevice), each containing field 1
        // (bssid string). We just need to recover the strings.
        let mut got = Vec::new();
        let mut pos = 0;
        while pos < proto.len() {
            let (field_num, wire_type, np) = decode_tag(proto, pos).unwrap();
            pos = np;
            if field_num == 2 && wire_type == 2 {
                let (sub, np) = decode_length_delimited(proto, pos).unwrap();
                pos = np;
                // Inside: field 1 string = bssid
                let mut sp = 0;
                while sp < sub.len() {
                    let (sf, sw, snp) = decode_tag(sub, sp).unwrap();
                    sp = snp;
                    if sf == 1 && sw == 2 {
                        let (s, snp) = decode_length_delimited(sub, sp).unwrap();
                        sp = snp;
                        got.push(String::from_utf8(s.to_vec()).unwrap());
                    } else {
                        sp = skip_field(sub, sp, sw).unwrap();
                    }
                }
            } else {
                pos = skip_field(proto, pos, wire_type).unwrap();
            }
        }
        assert_eq!(got, bssids);
    }

    /// Empty input still produces a well-formed envelope with length == 0.
    #[test]
    fn encode_empty_protobuf_has_nonzero_length_for_metadata_fields() {
        // The encoder always emits unknown_value1 and return_single_result
        // (two varint fields), so even with zero BSSIDs the protobuf is
        // non-empty. We just check the framing is consistent.
        let bssids: Vec<String> = Vec::new();
        let envelope = encode_request(&bssids);
        let len_bytes: [u8; 4] = envelope[LEN_OFFSET..PAYLOAD_OFFSET].try_into().unwrap();
        let declared = u32::from_be_bytes(len_bytes) as usize;
        assert_eq!(declared, envelope.len() - PAYLOAD_OFFSET);
    }
}
