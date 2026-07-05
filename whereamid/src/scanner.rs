//! Wi-Fi scanner: shells out to `iw dev <iface> scan trigger` then `iw dev <iface> scan dump`.

use anyhow::{bail, Context, Result};
use tokio::process::Command;
use tokio::time::{sleep, timeout, Duration};
use tracing::{debug, warn};

const IW_TIMEOUT: Duration = Duration::from_secs(10);

/// Default comma-separated value accepted by `--mobile-ssid-denylist`.
pub const DEFAULT_MOBILE_SSID_DENYLIST: &str =
    "DSB_Wi-Fi,.DSB_Wi-Fi,dsb-mng-wifi,MOVIA*,Bus_WiFi,Bus WiFi,*_iPhone,*_Hotspot,iPhone,AndroidAP*";

pub fn default_mobile_ssid_denylist() -> Vec<String> {
    DEFAULT_MOBILE_SSID_DENYLIST
        .split(',')
        .map(str::to_owned)
        .collect()
}

/// Return whether a scanner-observed SSID is classified as mobile.
///
/// Missing, empty, and the scanner sentinel `<hidden>` are always treated as
/// fixed, even when a broad pattern such as `*` would otherwise match. Patterns
/// are anchored globs: `*` matches zero or more Unicode scalar values and `?`
/// exactly one. Literal scalars compare by their locale-independent Unicode
/// lowercase expansions; this is not locale-sensitive collation or full
/// Unicode case folding.
pub fn is_mobile_ssid(ssid: Option<&str>, denylist: &[String]) -> bool {
    let Some(ssid) = ssid else {
        return false;
    };
    if ssid.is_empty() || ssid.eq_ignore_ascii_case("<hidden>") {
        return false;
    }

    denylist.iter().any(|pattern| glob_matches(pattern, ssid))
}

/// Match one anchored `*`/`?` glob case-insensitively. A dynamic-programming
/// row keeps matching total for arbitrary UTF-8 and avoids backtracking.
fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();

    let mut previous = vec![false; value.len() + 1];
    previous[0] = true;

    for token in pattern {
        let mut current = vec![false; value.len() + 1];
        if token == '*' {
            current[0] = previous[0];
        }
        for index in 1..=value.len() {
            current[index] = match token {
                '*' => previous[index] || current[index - 1],
                '?' => previous[index - 1],
                literal => {
                    previous[index - 1]
                        && literal.to_lowercase().eq(value[index - 1].to_lowercase())
                }
            };
        }
        previous = current;
    }

    previous[value.len()]
}

/// A single observed Wi-Fi network from a scan.
#[derive(Debug, Clone)]
pub struct ScannedNetwork {
    pub bssid: String,
    pub ssid: Option<String>,
    pub signal_dbm: i32,
    pub channel: Option<i32>,
    #[allow(dead_code)]
    pub frequency: Option<i32>,
}

/// Scan Wi-Fi networks. Tries nmcli first (no privileges needed since NetworkManager
/// runs as root), falls back to iw scan trigger/dump (needs CAP_NET_ADMIN).
pub async fn wifi_scan(interface: &str) -> Result<Vec<ScannedNetwork>> {
    // Try nmcli first — no special privileges needed
    match wifi_scan_nmcli(interface).await {
        Ok(networks) if !networks.is_empty() => return Ok(networks),
        Ok(_) => debug!("nmcli returned no networks, falling back to iw"),
        Err(e) => debug!("nmcli scan failed ({e}), falling back to iw"),
    }

    wifi_scan_iw(interface).await
}

/// Scan via NetworkManager CLI. Triggers rescan then lists results.
async fn wifi_scan_nmcli(interface: &str) -> Result<Vec<ScannedNetwork>> {
    // Request a rescan (non-blocking, may silently fail if too frequent)
    let _ = timeout(
        IW_TIMEOUT,
        Command::new("nmcli")
            .args(["device", "wifi", "rescan", "ifname", interface])
            // FREQ is human-readable even in terse mode; pin its unit suffix.
            .env("LC_ALL", "C")
            // task-0190: kill the child if this future is dropped mid-scan on
            // shutdown, so a cancelled scan leaves no orphaned nmcli process.
            .kill_on_drop(true)
            .output(),
    )
    .await;

    // Brief pause for scan to complete
    sleep(Duration::from_millis(1500)).await;

    // List results in terse machine-readable format
    let output = timeout(
        IW_TIMEOUT,
        Command::new("nmcli")
            .args([
                "-t",
                "-f",
                "BSSID,SSID,SIGNAL,CHAN,FREQ",
                "device",
                "wifi",
                "list",
                "ifname",
                interface,
            ])
            // Keep the parsed ` MHz` suffix stable across host locales.
            .env("LC_ALL", "C")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("nmcli wifi list timed out")?
    .context("failed to run nmcli")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("nmcli wifi list failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut networks = parse_nmcli_output(&stdout);
    networks.sort_by_key(|n| std::cmp::Reverse(n.signal_dbm));
    Ok(networks)
}

/// Parse nmcli terse output. In -t mode, nmcli escapes ':' in field values as '\:'.
/// The field separator is an unescaped ':'.
/// Fields: BSSID:SSID:SIGNAL:CHAN:FREQ
/// Under the C locale requested by `wifi_scan_nmcli`, NetworkManager renders
/// FREQ as an integer followed by the literal ` MHz` suffix.
/// Signal is 0-100 percentage, convert to approximate dBm.
pub fn parse_nmcli_output(output: &str) -> Vec<ScannedNetwork> {
    let mut networks = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Split on unescaped ':' (i.e. ':' not preceded by '\')
        let fields = split_nmcli_fields(line);
        if fields.len() < 5 {
            continue;
        }

        let bssid = fields[0].replace("\\:", ":");
        let ssid_raw = fields[1].replace("\\:", ":");
        let ssid = if ssid_raw.is_empty() {
            None
        } else {
            Some(ssid_raw)
        };

        // Skip rows where signal is missing or unparseable. The previous
        // behaviour fell back to 0%, which the linear conversion below would
        // then map to -90 dBm — indistinguishable from a real weak AP and
        // thus poisons trilateration weights with fake data.
        let signal_pct: i32 = match fields[2].trim().parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let channel: Option<i32> = fields[3].parse().ok();
        let freq = parse_nmcli_frequency(&fields[4]);

        // Convert signal percentage (0-100) to approximate dBm:
        // 100 ≈ -30 dBm, 0 ≈ -90 dBm via the linear map below.
        // NOTE: This is a coarse approximation. nmcli's percentage is itself
        // derived from RSSI by a non-linear, driver-dependent formula in
        // NetworkManager, so positions trilaterated purely from nmcli scans
        // will have systematically biased weights. iw reports raw dBm and
        // is the authoritative source when accuracy matters.
        let signal_dbm = -90 + ((signal_pct as f64 * 60.0 / 100.0) as i32);

        networks.push(ScannedNetwork {
            bssid: normalize_bssid(&bssid),
            ssid,
            signal_dbm,
            channel,
            frequency: freq,
        });
    }
    networks
}

/// Parse nmcli's C-locale `<integer> MHz` frequency representation.
///
/// Bare integers remain accepted for compatibility with older callers of the
/// public parser, but no other suffix or trailing content is accepted.
fn parse_nmcli_frequency(field: &str) -> Option<i32> {
    field.strip_suffix(" MHz").unwrap_or(field).parse().ok()
}

/// Split an nmcli -t line on unescaped ':' characters.
/// In nmcli terse mode, literal colons in values are escaped as '\:'.
pub fn split_nmcli_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            // nmcli terse mode escapes both the separator (`:` -> `\:`) and
            // the escape char itself (`\` -> `\\`). Consume the escaped char
            // so a literal backslash (`\\`) does not leave a dangling `\` that
            // re-triggers escape handling on the following byte (which would
            // mis-parse `\\:` into a field containing `\:`). task-0226.
            match chars.peek() {
                Some(&':') => {
                    current.push(':');
                    chars.next();
                }
                Some(&'\\') => {
                    current.push('\\');
                    chars.next();
                }
                // Lone/other backslash: emit it verbatim, best-effort.
                _ => current.push('\\'),
            }
        } else if ch == ':' {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    fields.push(current);
    fields
}

/// Scan via iw (needs CAP_NET_ADMIN for trigger). Falls back to cached dump.
async fn wifi_scan_iw(interface: &str) -> Result<Vec<ScannedNetwork>> {
    // Trigger a background scan
    let mut retries = 3;
    loop {
        let output = timeout(
            IW_TIMEOUT,
            Command::new("iw")
                .args(["dev", interface, "scan", "trigger"])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .context("iw scan trigger timed out")?
        .context("failed to run iw scan trigger")?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        if output.status.success() {
            break;
        } else if stderr.contains("Device or resource busy") && retries > 0 {
            retries -= 1;
            debug!("scan trigger busy, retrying in 1s ({retries} retries left)");
            sleep(Duration::from_secs(1)).await;
        } else if stderr.contains("command failed: Network is down") {
            bail!("wifi interface {interface} is down");
        } else {
            warn!("scan trigger failed: {stderr}");
            break;
        }
    }

    // Brief pause for scan to complete
    sleep(Duration::from_millis(500)).await;

    // Dump results
    let output = timeout(
        IW_TIMEOUT,
        Command::new("iw")
            .args(["dev", interface, "scan", "dump"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("iw scan dump timed out")?
    .context("failed to run iw scan dump")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("iw scan dump failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut networks = parse_iw_output(&stdout);
    networks.sort_by_key(|n| std::cmp::Reverse(n.signal_dbm));
    Ok(networks)
}

/// Parse the output of `iw dev <iface> scan dump`.
pub fn parse_iw_output(output: &str) -> Vec<ScannedNetwork> {
    let mut networks = Vec::new();
    let mut current_bssid: Option<String> = None;
    let mut current_ssid: Option<String> = None;
    let mut current_signal: Option<i32> = None;
    let mut current_channel: Option<i32> = None;
    let mut current_freq: Option<i32> = None;

    for line in output.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("BSS ") {
            // Save previous entry
            if let (Some(bssid), Some(signal)) = (current_bssid.take(), current_signal.take()) {
                networks.push(ScannedNetwork {
                    bssid: normalize_bssid(&bssid),
                    ssid: current_ssid.take(),
                    signal_dbm: signal,
                    channel: current_channel.take(),
                    frequency: current_freq.take(),
                });
            } else {
                current_ssid = None;
                current_channel = None;
                current_freq = None;
            }

            // Extract BSSID from "BSS aa:bb:cc:dd:ee:ff(on wlan0)" or similar
            let bssid_part = rest.split('(').next().unwrap_or(rest).trim();
            current_bssid = Some(bssid_part.to_string());
            current_signal = None;
        } else if let Some(rest) = line.strip_prefix("signal:") {
            // "signal: -65.00 dBm"
            let signal_str = rest.split_whitespace().next().unwrap_or("");
            if let Ok(signal) = signal_str.parse::<f64>() {
                current_signal = Some(signal as i32);
            }
        } else if let Some(rest) = line.strip_prefix("SSID:") {
            let ssid = rest.trim();
            if !ssid.is_empty() {
                current_ssid = Some(ssid.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("DS Parameter set: channel") {
            if let Ok(ch) = rest.trim().parse::<i32>() {
                current_channel = Some(ch);
            }
        } else if let Some(rest) = line.strip_prefix("freq:") {
            if let Ok(freq) = rest.trim().parse::<i32>() {
                current_freq = Some(freq);
                // Derive channel from frequency if not explicitly set
                if current_channel.is_none() {
                    current_channel = freq_to_channel(freq);
                }
            }
        }
    }

    // Don't forget the last entry
    if let (Some(bssid), Some(signal)) = (current_bssid, current_signal) {
        networks.push(ScannedNetwork {
            bssid: normalize_bssid(&bssid),
            ssid: current_ssid,
            signal_dbm: signal,
            channel: current_channel,
            frequency: current_freq,
        });
    }

    networks
}

/// Normalize BSSID to uppercase, colon-separated.
pub fn normalize_bssid(bssid: &str) -> String {
    bssid.trim().to_uppercase()
}

/// Convert Wi-Fi frequency (MHz) to channel number.
///
/// Covers 2.4 GHz (channels 1-14), 5 GHz (channels ~36-177), and Wi-Fi 6E /
/// 6 GHz (channels 1-233 starting at 5955 MHz). Per IEEE 802.11ax/be the 6 GHz
/// channels use formula `(freq - 5950) / 5` over `5955..=7115` MHz, except for
/// channel 2 at 5935 MHz, which is the single special case.
fn freq_to_channel(freq: i32) -> Option<i32> {
    match freq {
        2412 => Some(1),
        2417 => Some(2),
        2422 => Some(3),
        2427 => Some(4),
        2432 => Some(5),
        2437 => Some(6),
        2442 => Some(7),
        2447 => Some(8),
        2452 => Some(9),
        2457 => Some(10),
        2462 => Some(11),
        2467 => Some(12),
        2472 => Some(13),
        2484 => Some(14),
        // 6 GHz (Wi-Fi 6E / 7) — checked BEFORE the 5 GHz formula so that
        // any future widening of the 5 GHz range cannot silently shadow it.
        5935 => Some(2),
        f if (5955..=7115).contains(&f) => Some((f - 5950) / 5),
        // 5 GHz: channel = (freq - 5000) / 5. Range 5180-5885 covers UNII-1/2/3.
        // The gap 5895-5945 (UNII-4 / DSRC) stays None except for 5935 MHz
        // above, the isolated 6 GHz channel-2 special case.
        f if (5180..=5885).contains(&f) => Some((f - 5000) / 5),
        _ => None,
    }
}

/// Convert scan results to a ScanSample for the debounce ring buffer.
/// Preserves SSID, channel, and signal for each AP.
pub fn scan_to_sample(networks: &[ScannedNetwork]) -> crate::debounce::ScanSample {
    networks
        .iter()
        .map(|n| {
            (
                n.bssid.clone(),
                crate::debounce::ScanEntry {
                    signal_dbm: n.signal_dbm,
                    ssid: n.ssid.clone(),
                    channel: n.channel,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterns(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn mobile_movia_glob_is_anchored_and_case_insensitive() {
        let denylist = patterns(&["MOVIA*"]);
        assert!(is_mobile_ssid(Some("MOVIA_BUS_42"), &denylist));
        assert!(is_mobile_ssid(Some("movia-night"), &denylist));
        assert!(!is_mobile_ssid(Some("mymoviabus"), &denylist));
    }

    #[test]
    fn mobile_question_mark_matches_one_unicode_scalar() {
        let denylist = patterns(&["Bus_?"]);
        assert!(is_mobile_ssid(Some("bus_4"), &denylist));
        assert!(is_mobile_ssid(Some("BUS_å"), &denylist));
        assert!(!is_mobile_ssid(Some("Bus_42"), &denylist));
        assert!(!is_mobile_ssid(Some("Bus_"), &denylist));
    }

    #[test]
    fn mobile_missing_empty_and_hidden_are_never_flagged() {
        let denylist = patterns(&["*"]);
        assert!(!is_mobile_ssid(None, &denylist));
        assert!(!is_mobile_ssid(Some(""), &denylist));
        assert!(!is_mobile_ssid(Some("<hidden>"), &denylist));
        assert!(!is_mobile_ssid(Some("<HIDDEN>"), &denylist));
    }

    #[test]
    fn mobile_glob_specials_are_limited_to_star_and_question_mark() {
        let denylist = patterns(&["bus[0-9]"]);
        assert!(is_mobile_ssid(Some("BUS[0-9]"), &denylist));
        assert!(!is_mobile_ssid(Some("bus4"), &denylist));
    }

    #[test]
    fn test_parse_iw_output() {
        let output = r#"BSS aa:bb:cc:dd:ee:ff(on wlan0)
	freq: 2437
	signal: -65.00 dBm
	SSID: TestWiFi
	DS Parameter set: channel 6
BSS 11:22:33:44:55:66(on wlan0)
	freq: 5180
	signal: -78.00 dBm
	SSID: eduroam
"#;
        let networks = parse_iw_output(output);
        assert_eq!(networks.len(), 2);
        assert_eq!(networks[0].bssid, "AA:BB:CC:DD:EE:FF");
        assert_eq!(networks[0].ssid, Some("TestWiFi".to_string()));
        assert_eq!(networks[0].signal_dbm, -65);
        assert_eq!(networks[0].channel, Some(6));
        assert_eq!(networks[0].frequency, Some(2437));

        assert_eq!(networks[1].bssid, "11:22:33:44:55:66");
        assert_eq!(networks[1].signal_dbm, -78);
    }

    #[test]
    fn test_parse_nmcli_output() {
        // nmcli -t escapes colons in BSSIDs as \:
        let output = "AA\\:BB\\:CC\\:DD\\:EE\\:01:TestNet_2G:65:11:2462 MHz\nAA\\:BB\\:CC\\:DD\\:EE\\:02:TestNet_5G:49:100:5500 MHz\n";
        let networks = parse_nmcli_output(output);
        assert_eq!(networks.len(), 2);
        assert_eq!(networks[0].bssid, "AA:BB:CC:DD:EE:01");
        assert_eq!(networks[0].ssid, Some("TestNet_2G".to_string()));
        assert_eq!(networks[0].channel, Some(11));
        assert_eq!(networks[0].frequency, Some(2462));
        // signal: 65% -> -90 + (65*60/100) = -90 + 39 = -51
        assert_eq!(networks[0].signal_dbm, -51);
        assert_eq!(networks[1].bssid, "AA:BB:CC:DD:EE:02");
        assert_eq!(networks[1].ssid, Some("TestNet_5G".to_string()));
        assert_eq!(networks[1].channel, Some(100));
        assert_eq!(networks[1].frequency, Some(5500));
    }

    #[test]
    fn test_parse_nmcli_skips_empty_signal() {
        // First row has no signal field; second row has it. Only the second
        // row should be emitted — empty signal must NOT be coerced to 0%/-90 dBm.
        let output = concat!(
            "AA\\:BB\\:CC\\:DD\\:EE\\:01:NoSig::6:2437 MHz\n",
            "AA\\:BB\\:CC\\:DD\\:EE\\:02:HasSig:50:6:2437 MHz\n",
        );
        let networks = parse_nmcli_output(output);
        assert_eq!(
            networks.len(),
            1,
            "row with empty signal must be skipped, got {networks:?}"
        );
        assert_eq!(networks[0].bssid, "AA:BB:CC:DD:EE:02");
    }

    #[test]
    fn test_parse_nmcli_skips_garbage_signal() {
        // Non-numeric signal field also skipped, not coerced to 0.
        let output = "AA\\:BB\\:CC\\:DD\\:EE\\:03:Bad:not_a_number:6:2437 MHz\n";
        let networks = parse_nmcli_output(output);
        assert!(
            networks.is_empty(),
            "row with garbage signal must be skipped, got {networks:?}"
        );
    }

    #[test]
    fn test_split_nmcli_fields() {
        let line = "AA\\:BB\\:CC\\:DD\\:EE\\:FF:MySSID:75:6:2437 MHz";
        let fields = split_nmcli_fields(line);
        assert_eq!(
            fields,
            vec!["AA:BB:CC:DD:EE:FF", "MySSID", "75", "6", "2437 MHz"]
        );
    }

    #[test]
    fn test_parse_nmcli_keeps_rows_with_malformed_frequency() {
        let output = concat!(
            "AA\\:BB\\:CC\\:DD\\:EE\\:01:WrongUnit:50:100:5500 GHz\n",
            "AA\\:BB\\:CC\\:DD\\:EE\\:02:MissingSpace:50:100:5500MHz\n",
            "AA\\:BB\\:CC\\:DD\\:EE\\:03:TrailingGarbage:50:100:5500 MHz extra\n",
        );
        let networks = parse_nmcli_output(output);
        assert_eq!(networks.len(), 3);
        assert!(networks.iter().all(|network| network.frequency.is_none()));

        // Preserve the parser's previous bare-integer contract for callers
        // that supply synthetic or older nmcli output.
        assert_eq!(parse_nmcli_frequency("5500"), Some(5500));
    }

    /// task-0226: nmcli escapes `\` as `\\`. A field ending in a literal
    /// backslash (`SSID\` -> wire `SSID\\`) followed by the separator must
    /// yield the backslash and a clean split, NOT leave a dangling `\:`
    /// escape. The `.*` proptest `split_nmcli_no_unescaped_backslash_colon`
    /// catches this flakily; this pins it deterministically.
    #[test]
    fn split_nmcli_handles_escaped_backslash() {
        // Wire: field "a\" (a + literal backslash) then separator then "b".
        // "a" + "\\" (escaped backslash) + ":" (separator) + "b"
        assert_eq!(split_nmcli_fields("a\\\\:b"), vec!["a\\", "b"]);
        // Escaped backslash immediately followed by an escaped colon:
        // "\\" -> "\", "\:" -> ":", all one field.
        assert_eq!(split_nmcli_fields("\\\\\\:"), vec!["\\:"]);
        // No field may retain an unprocessed "\:" escape.
        for f in split_nmcli_fields("a\\\\:b\\:c") {
            assert!(!f.contains("\\:"), "unprocessed escape in {f:?}");
        }
    }

    #[test]
    fn test_normalize_bssid() {
        assert_eq!(normalize_bssid("aa:bb:cc:dd:ee:ff"), "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn freq_to_channel_band_boundaries() {
        let cases = [
            // 2.4 GHz: frequencies are discrete rather than a continuous range.
            (2407, None, "below 2.4 GHz"),
            (2411, None, "immediately below 2.4 GHz"),
            (2412, Some(1), "first 2.4 GHz channel"),
            (2437, Some(6), "interior 2.4 GHz channel"),
            (2484, Some(14), "last 2.4 GHz channel"),
            (2485, None, "immediately above 2.4 GHz"),
            (2489, None, "above 2.4 GHz"),
            // Pin the first two 5 GHz channels so moving the lower guard from
            // 5180 to 5185 cannot slip through as an off-by-one regression.
            (5175, None, "below 5 GHz"),
            (5179, None, "immediately below 5 GHz"),
            (5180, Some(36), "first 5 GHz channel"),
            (5185, Some(37), "second 5 GHz channel"),
            (5825, Some(165), "interior 5 GHz channel"),
            (5885, Some(177), "last 5 GHz channel"),
            (5886, None, "immediately above 5 GHz"),
            (5890, None, "above 5 GHz"),
            (5905, None, "UNII-4 gap"),
            // 6 GHz / Wi-Fi 6E. Channel 2 at 5935 MHz is isolated from the
            // formula-based range that starts with channel 1 at 5955 MHz.
            (5934, None, "immediately below special-case channel 2"),
            (5935, Some(2), "special-case 6 GHz channel 2"),
            (5936, None, "immediately above special-case channel 2"),
            (5950, None, "below 6 GHz"),
            (5954, None, "immediately below 6 GHz"),
            (5955, Some(1), "first 6 GHz channel"),
            (6105, Some(31), "interior 6 GHz channel"),
            (7115, Some(233), "last 6 GHz channel"),
            (7116, None, "immediately above 6 GHz"),
            (7120, None, "above 6 GHz"),
        ];

        for (frequency, expected, description) in cases {
            assert_eq!(
                freq_to_channel(frequency),
                expected,
                "unexpected mapping at {description} ({frequency} MHz)"
            );
        }
    }

    #[test]
    fn freq_to_channel_outputs_are_valid_channels() {
        // Frequencies are parsed as MHz. Exhausting u16 covers the complete
        // practical non-negative domain and every branch that can return Some,
        // while keeping this regression property deterministic and fast.
        for frequency in 0..=u16::MAX {
            if let Some(channel) = freq_to_channel(i32::from(frequency)) {
                assert!(
                    (1..=233).contains(&channel),
                    "{frequency} MHz mapped to invalid channel {channel}"
                );
            }
        }
    }
}

// task-0082: Kani harnesses. See apple.rs for setup notes.
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// `freq_to_channel` is a total function — every i32 must produce
    /// either Some or None, never panic. Kani exhaustively explores the
    /// branch / range guards (no loops, no unwind needed).
    #[kani::proof]
    fn freq_to_channel_total() {
        let freq: i32 = kani::any();
        let _ = freq_to_channel(freq);
    }

    /// `split_nmcli_fields` walks bytes once, handling backslash escapes.
    /// Bounded UTF-8 input; Kani proves panic-freedom (no out-of-bounds
    /// peeks, no integer panics) over every valid 32-byte UTF-8 string.
    #[kani::proof]
    #[kani::unwind(33)]
    fn split_nmcli_fields_no_panic() {
        const N: usize = 32;
        let bytes: [u8; N] = kani::any();
        if let Ok(s) = std::str::from_utf8(&bytes) {
            let _ = split_nmcli_fields(s);
        }
    }

    /// `normalize_bssid` is .trim().to_uppercase() — a thin wrapper
    /// around stdlib but still worth proving panic-free under symbolic
    /// UTF-8 input.
    #[kani::proof]
    #[kani::unwind(33)]
    fn normalize_bssid_no_panic() {
        const N: usize = 32;
        let bytes: [u8; N] = kani::any();
        if let Ok(s) = std::str::from_utf8(&bytes) {
            let _ = normalize_bssid(s);
        }
    }
}
