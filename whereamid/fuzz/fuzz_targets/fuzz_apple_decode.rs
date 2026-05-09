#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // decode_response expects the data after the 10-byte header.
    // Feed raw bytes — the decoder must handle arbitrary garbage gracefully.
    let dummy_requested = vec!["AA:BB:CC:DD:EE:FF".to_string()];
    let _ = whereamid::apple::decode_response(data, &dummy_requested);

    // Also fuzz the sub-parsers directly
    let _ = whereamid::apple::parse_wifi_device(data);
    let _ = whereamid::apple::parse_location(data);
});
