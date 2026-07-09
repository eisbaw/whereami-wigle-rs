//! Relative duration specs: parse strings like "7d", "24h", "30m", "1w" into
//! an absolute (start, end) time window ending at "now".
//!
//! This is deliberately generic and NOT history-specific: it lives here rather
//! than in `history.rs` so a future sibling feature (e.g. a stats-over-time
//! endpoint) can reuse it without depending on the history module's public API.
//! Extracted from `history.rs` in task-0137 while there was a single caller.

use anyhow::{anyhow, Result};
use chrono::{DateTime, TimeDelta, Utc};

/// Parse a relative range string like "7d", "24h", "30m" into (start, end)
/// where end is now and start is now-duration. Also accepts "1w".
pub(crate) fn parse_range(spec: &str) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(anyhow!("empty range spec"));
    }
    // Split off the LAST CHARACTER (not last byte) so non-ASCII unit chars
    // don't panic on UTF-8 boundaries. task-0048.
    let last_char = spec
        .chars()
        .next_back()
        .ok_or_else(|| anyhow!("empty range spec"))?;
    let unit_len = last_char.len_utf8();
    let (num_str, unit) = spec.split_at(spec.len() - unit_len);
    let n: i64 = num_str
        .parse()
        .map_err(|_| anyhow!("invalid range '{spec}': expected '<number><unit>' (e.g. '7d')"))?;
    if n <= 0 {
        return Err(anyhow!("range '{spec}' must be > 0"));
    }
    // Use the fallible `try_*` constructors: `TimeDelta::days(n)` and friends
    // PANIC when the requested magnitude exceeds `TimeDelta`'s bounds (e.g.
    // ~10^17 seconds). `try_*` returns None instead so we can surface the
    // protocol-mandated error JSON rather than aborting the connection task.
    // task-0184.
    let dur = match unit {
        "s" => TimeDelta::try_seconds(n),
        "m" => TimeDelta::try_minutes(n),
        "h" => TimeDelta::try_hours(n),
        "d" => TimeDelta::try_days(n),
        "w" => TimeDelta::try_weeks(n),
        other => {
            return Err(anyhow!(
                "unknown range unit '{other}' in '{spec}': use s/m/h/d/w"
            ))
        }
    }
    .ok_or_else(|| anyhow!("range '{spec}' is too large to represent"))?;
    // Even when `dur` is a valid TimeDelta, `now - dur` PANICS when the
    // resulting year underflows `DateTime`'s representable range (~year
    // -262144). e.g. "100000000d" (~274k years) constructs a valid TimeDelta
    // but overflows the subtraction. `checked_sub_signed` returns None there.
    // task-0184.
    let now = Utc::now();
    let start = now
        .checked_sub_signed(dur)
        .ok_or_else(|| anyhow!("range '{spec}' extends before the earliest representable date"))?;
    Ok((start, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_range_understands_common_units() {
        let now_ts = Utc::now().timestamp();
        for spec in ["7d", "24h", "30m", "1w", "300s"] {
            let (start, end) = parse_range(spec).unwrap();
            assert!((end.timestamp() - now_ts).abs() < 5, "end ≈ now");
            assert!(start < end, "start {start:?} must be before end {end:?}");
        }
    }

    #[test]
    fn parse_range_rejects_garbage() {
        assert!(parse_range("").is_err());
        assert!(parse_range("7").is_err()); // no unit
        assert!(parse_range("xd").is_err());
        assert!(parse_range("7y").is_err()); // unknown unit
        assert!(parse_range("0d").is_err()); // must be > 0
        assert!(parse_range("-3d").is_err());
    }

    /// task-0162: internal whitespace is NOT tolerated. `parse_range` splits
    /// off only the LAST char as the unit, so "7 d" yields num_str="7 "
    /// (trailing space) which `i64::parse` rejects. Pin that the current
    /// behavior is a clean Err naming the offending spec — a future parser
    /// that "helpfully" trimmed interior whitespace would silently change this.
    #[test]
    fn parse_range_rejects_internal_whitespace() {
        let err = parse_range("7 d").unwrap_err().to_string();
        assert!(
            err.contains("invalid range '7 d'"),
            "internal-whitespace spec must Err naming '7 d', got: {err}"
        );
        // A leading interior space before the digits is equally rejected.
        assert!(parse_range("1 2d").is_err());
    }

    /// task-0048: parse_range used split_at(byte_len - 1) which panics on
    /// non-char-boundary inputs. Now uses last char's len_utf8.
    #[test]
    fn parse_range_no_panic_on_non_ascii() {
        // Multi-byte unit char: '日' is 3 bytes. Must NOT panic.
        assert!(parse_range("7日").is_err());
        // 4-byte char (emoji).
        assert!(parse_range("7🚀").is_err());
        // Pure non-ASCII string.
        assert!(parse_range("日").is_err());
        // Whitespace-only.
        assert!(parse_range("   ").is_err());
    }

    /// task-0184: parse_range fed the count into panicking chrono APIs.
    /// Two distinct overflow sites must both degrade to Err (never panic):
    ///
    ///  1. TimeDelta construction: a magnitude beyond TimeDelta's bounds
    ///     (~10^17 s) made `Duration::seconds` panic.
    ///  2. DateTime subtraction: a magnitude that builds a valid TimeDelta
    ///     but drives the year below ~-262144 made `now - dur` panic.
    ///
    /// `100000000d` (~274k years, 10 bytes — well inside the 64 KiB request
    /// cap) is the reported reachable-from-TCP panic.
    #[test]
    fn parse_range_returns_err_on_overflow_never_panics() {
        // Site 2 (DateTime subtraction): valid TimeDelta, year underflow.
        assert!(
            parse_range("100000000d").is_err(),
            "100000000d must be a clean Err, not a panic"
        );
        // Site 1 (TimeDelta construction): magnitude beyond TimeDelta bounds.
        assert!(
            parse_range("99999999999999999s").is_err(),
            "99999999999999999s must be a clean Err, not a panic"
        );

        // i64::MAX against every unit — the largest magnitude the parser can
        // accept. Whichever site each unit trips, the result must be Err.
        let max = i64::MAX; // 9223372036854775807
        for unit in ["s", "m", "h", "d", "w"] {
            let spec = format!("{max}{unit}");
            assert!(
                parse_range(&spec).is_err(),
                "parse_range({spec:?}) must be Err (overflow), not a panic"
            );
        }

        // A weeks value that constructs a valid TimeDelta yet overflows the
        // subtraction: 10^8 weeks ≈ 1.9M years into the past (well beyond
        // DateTime's ~-262144 floor). 10^7 weeks (~191k years) would still be
        // representable, so it must be large enough to actually cross it.
        assert!(
            parse_range("100000000w").is_err(),
            "100000000w must overflow the date subtraction to Err"
        );
    }
}

// task-0082: Kani harnesses. See apple.rs for setup notes.
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// `parse_range` had a real panic bug (task-0048: byte-indexed split_at
    /// on a non-char-boundary). Kani proves panic-freedom over every
    /// bounded UTF-8 input — including the multi-byte char cases the
    /// existing unit tests can only sample.
    #[kani::proof]
    #[kani::unwind(33)]
    fn parse_range_no_panic() {
        const N: usize = 32;
        let bytes: [u8; N] = kani::any();
        if let Ok(s) = std::str::from_utf8(&bytes) {
            let _ = parse_range(s);
        }
    }
}
