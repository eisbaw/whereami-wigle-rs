#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let input = String::from_utf8_lossy(data);

    // Fuzz the field splitter directly
    let _ = whereamid::scanner::split_nmcli_fields(&input);

    // Fuzz the full parser
    let _ = whereamid::scanner::parse_nmcli_output(&input);
});
