//! Wi-Fi scanner: shells out to `iw dev <iface> scan trigger` then `iw dev <iface> scan dump`.

use anyhow::{bail, Context, Result};
use tokio::process::Command;
use tokio::time::{sleep, timeout, Duration};
use tracing::{debug, warn};

const IW_TIMEOUT: Duration = Duration::from_secs(10);

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
    networks.sort_by(|a, b| b.signal_dbm.cmp(&a.signal_dbm));
    Ok(networks)
}

/// Parse nmcli terse output. In -t mode, nmcli escapes ':' in field values as '\:'.
/// The field separator is an unescaped ':'.
/// Fields: BSSID:SSID:SIGNAL:CHAN:FREQ
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
        let signal_pct: i32 = fields[2].parse().unwrap_or(0);
        let channel: Option<i32> = fields[3].parse().ok();
        let freq: Option<i32> = fields[4].parse().ok();

        // Convert signal percentage (0-100) to approximate dBm
        // nmcli reports 0-100 where 100 ~ -30 dBm, 0 ~ -90 dBm
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

/// Split an nmcli -t line on unescaped ':' characters.
/// In nmcli terse mode, literal colons in values are escaped as '\:'.
pub fn split_nmcli_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                if next == ':' {
                    current.push(':');
                    chars.next();
                } else {
                    current.push(ch);
                }
            } else {
                current.push(ch);
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
    networks.sort_by(|a, b| b.signal_dbm.cmp(&a.signal_dbm));
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
        // 5 GHz: channel = (freq - 5000) / 5
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
        let output = "AA\\:BB\\:CC\\:DD\\:EE\\:01:TestNet_2G:65:11:2462\nAA\\:BB\\:CC\\:DD\\:EE\\:02:TestNet_5G:49:100:5500\n";
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
    }

    #[test]
    fn test_split_nmcli_fields() {
        let line = "AA\\:BB\\:CC\\:DD\\:EE\\:FF:MySSID:75:6:2437";
        let fields = split_nmcli_fields(line);
        assert_eq!(
            fields,
            vec!["AA:BB:CC:DD:EE:FF", "MySSID", "75", "6", "2437"]
        );
    }

    #[test]
    fn test_normalize_bssid() {
        assert_eq!(normalize_bssid("aa:bb:cc:dd:ee:ff"), "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn test_freq_to_channel() {
        assert_eq!(freq_to_channel(2437), Some(6));
        assert_eq!(freq_to_channel(5180), Some(36));
    }
}
