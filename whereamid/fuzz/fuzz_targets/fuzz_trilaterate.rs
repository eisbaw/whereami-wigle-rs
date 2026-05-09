#![no_main]
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use whereamid::trilaterate::{filter_outliers, trilaterate, PositionedAp};

/// Fuzzer-friendly AP representation that Arbitrary can derive.
/// Constrains values to plausible ranges to avoid NaN/Inf poisoning
/// the haversine math — we want to find logic bugs, not IEEE754 edge cases.
#[derive(Arbitrary, Debug)]
struct FuzzAp {
    /// Latitude in degrees, will be clamped to [-90, 90]
    lat_raw: i32,
    /// Longitude in degrees, will be clamped to [-180, 180]
    lon_raw: i32,
    /// Signal in dBm, typically [-100, 0]
    signal_raw: i8,
    has_signal: bool,
}

impl FuzzAp {
    fn to_positioned_ap(&self) -> PositionedAp {
        // Map i32 range to valid lat/lon via modular arithmetic
        let lat = (self.lat_raw % 18001) as f64 / 200.0; // [-90.005, 90.005]
        let lon = (self.lon_raw % 36001) as f64 / 200.0; // [-180.005, 180.005]
        let signal = if self.has_signal {
            Some(self.signal_raw.clamp(-100, 0) as i32)
        } else {
            None
        };
        PositionedAp {
            lat,
            lon,
            signal_dbm: signal,
        }
    }
}

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    aps: Vec<FuzzAp>,
}

fuzz_target!(|input: FuzzInput| {
    // Limit to a reasonable number of APs to keep execution fast
    if input.aps.is_empty() || input.aps.len() > 200 {
        return;
    }

    let positioned: Vec<PositionedAp> = input.aps.iter().map(|a| a.to_positioned_ap()).collect();

    // filter_outliers must not panic
    let filtered = filter_outliers(&positioned);
    assert!(!filtered.is_empty() || positioned.len() <= 2);

    // trilaterate must not panic (it returns Result, so Err is fine)
    let _ = trilaterate(&positioned);
});
