#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // parse_iw_output takes &str, so only valid UTF-8 is interesting.
    // Using from_utf8_lossy mirrors what the real code does with command output.
    let input = String::from_utf8_lossy(data);
    let _ = whereamid::scanner::parse_iw_output(&input);
});
