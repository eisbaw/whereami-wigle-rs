#![no_main]
//! Fuzz the Apple WPS protobuf ENCODER.
//!
//! Companion to fuzz_apple_decode (which exercises arbitrary garbage input
//! to the parser side). This target generates *structured* BSSID lists and
//! asserts the encoder's invariants:
//!   1. encode_request never panics for any input.
//!   2. The u32 BE payload-length field at offset 46 of the envelope
//!      exactly equals the protobuf payload length (i.e. envelope.len() - 50).
//!      This is the regression check for task-0019: the prior implementation
//!      truncated the length to a single u8.
//!   3. The encoded envelope round-trips through decode_response without
//!      panicking (it may return Err for empty inputs, but must not crash).

use libfuzzer_sys::fuzz_target;

/// Apple ARPC envelope header layout (big-endian, all bytes counted):
/// - u16 version              (2)
/// - u16 locale_len + bytes   (2 + 5  = 7)
/// - u16 app_id_len + bytes   (2 + 19 = 21)
/// - u16 os_version_len + bytes (2 + 10 = 12)
/// - u32 flag                 (4)
/// - u32 payload_len          (4)   <-- starts here
/// - payload                  (variable)
const PAYLOAD_LEN_OFFSET: usize = 2 + 7 + 21 + 12 + 4; // 46
const PAYLOAD_OFFSET: usize = PAYLOAD_LEN_OFFSET + 4; // 50

fuzz_target!(|data: &[u8]| {
    // Treat fuzz input as a sequence of 6-byte BSSIDs. Cap at 1024 BSSIDs to
    // keep iterations fast; the truncation bug shows up well before that.
    let bssids: Vec<String> = data
        .chunks_exact(6)
        .take(1024)
        .map(|chunk| {
            format!(
                "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5]
            )
        })
        .collect();

    let envelope = whereamid::apple::encode_request(&bssids);

    // Invariant: envelope is at least the fixed header.
    assert!(
        envelope.len() >= PAYLOAD_OFFSET,
        "envelope shorter than fixed header: {} bytes",
        envelope.len()
    );

    // Invariant: payload_len field == actual payload length.
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&envelope[PAYLOAD_LEN_OFFSET..PAYLOAD_LEN_OFFSET + 4]);
    let payload_len = u32::from_be_bytes(buf) as usize;
    let actual_payload = envelope.len() - PAYLOAD_OFFSET;
    assert_eq!(
        payload_len, actual_payload,
        "payload_len field ({payload_len}) does not match actual payload ({actual_payload})"
    );

    // Sanity: the encoded payload should be parseable by the decoder
    // without panicking. We pass `&bssids` as the requested list so the
    // decoder can attribute results.
    let _ = whereamid::apple::decode_response(&envelope, &bssids);
});
