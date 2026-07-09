//! Debounce: sliding window stable AP detection.
//!
//! Maintains a ring buffer of the last N scan samples. A BSSID is "stable"
//! when it appears in >= M of those N samples.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use whereami_protocol::Channel;

use crate::boot_clock::BootTime;

/// Per-AP info stored in each scan sample.
#[derive(Debug, Clone)]
pub struct ScanEntry {
    pub signal_dbm: i32,
    /// Provenance of `signal_dbm` (iw authoritative vs nmcli biased
    /// linearization), carried from the scanner so it survives the debounce
    /// ring to trilateration. `signal_dbm` stays `i32`. task-0126.
    pub rssi_source: crate::rssi::RssiSource,
    pub ssid: Option<String>,
    pub channel: Option<Channel>,
}

/// A single scan sample: maps BSSID -> scan entry.
pub type ScanSample = HashMap<String, ScanEntry>;

/// A timestamped scan sample.
pub struct TimestampedScan {
    pub sample: ScanSample,
    /// When this batch was received by the daemon. Used only for scan/debug
    /// latency display; autonomous freshness uses suspend-inclusive evidence.
    pub received_at: Instant,
    pub wall_clock: chrono::DateTime<chrono::Utc>,
    /// Start of the radio refresh attempt that produced this batch. `None`
    /// means the backend returned readable cached output without proving that
    /// a new refresh succeeded.
    pub confirmed_refresh_started_at: Option<BootTime>,
}

/// Debounce ring buffer for stable AP detection.
pub struct Debouncer {
    ring: VecDeque<TimestampedScan>,
    window: usize,
    threshold: usize,
}

impl Debouncer {
    /// Create a new debouncer with the given window size and threshold.
    pub fn new(window: usize, threshold: usize) -> Self {
        assert!(threshold <= window, "threshold must be <= window");
        Self {
            ring: VecDeque::with_capacity(window),
            window,
            threshold,
        }
    }

    /// Push a new scan sample into the ring buffer. Drops the oldest if at capacity.
    /// Direct callers have no radio-refresh proof, so this convenience path is
    /// deliberately unconfirmed. The scanner loop uses the evidence-aware API.
    #[allow(dead_code)]
    pub fn push_scan(&mut self, sample: ScanSample) {
        self.push_scan_with_refresh_evidence(sample, None);
    }

    /// Atomically retain one sample and its optional confirmed-refresh proof.
    pub fn push_scan_with_refresh_evidence(
        &mut self,
        sample: ScanSample,
        confirmed_refresh_started_at: Option<BootTime>,
    ) {
        self.push_scan_at(
            sample,
            Instant::now(),
            chrono::Utc::now(),
            confirmed_refresh_started_at,
        );
    }

    fn push_scan_at(
        &mut self,
        sample: ScanSample,
        received_at: Instant,
        wall_clock: chrono::DateTime<chrono::Utc>,
        confirmed_refresh_started_at: Option<BootTime>,
    ) {
        if self.ring.len() >= self.window {
            self.ring.pop_front();
        }
        self.ring.push_back(TimestampedScan {
            sample,
            received_at,
            wall_clock,
            confirmed_refresh_started_at,
        });
    }

    /// Forget every observed network and timestamp. Used by the confirmed
    /// privacy wipe while it owns the daemon-wide exclusive publication gate.
    pub fn clear(&mut self) {
        self.ring.clear();
    }

    /// Check if a specific BSSID is stable (appears in >= threshold samples).
    #[allow(dead_code)]
    pub fn is_stable(&self, bssid: &str) -> bool {
        let count = self
            .ring
            .iter()
            .filter(|s| s.sample.contains_key(bssid))
            .count();
        count >= self.threshold
    }

    /// Return all currently stable BSSIDs.
    pub fn stable_bssids(&self) -> HashSet<String> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for ts in &self.ring {
            for bssid in ts.sample.keys() {
                *counts.entry(bssid.as_str()).or_insert(0) += 1;
            }
        }

        counts
            .into_iter()
            .filter(|(_, count)| *count >= self.threshold)
            .map(|(bssid, _)| bssid.to_string())
            .collect()
    }

    /// Get the signal reading for a BSSID from the most recent scan only.
    /// Returns None if the BSSID was not in the latest scan.
    #[allow(dead_code)]
    pub fn latest_signal(&self, bssid: &str) -> Option<i32> {
        self.ring
            .back()
            .and_then(|ts| ts.sample.get(bssid).map(|e| e.signal_dbm))
    }

    /// Get the full scan entry for a BSSID from the most recent scan.
    #[allow(dead_code)]
    pub fn latest_entry(&self, bssid: &str) -> Option<&ScanEntry> {
        self.ring.back().and_then(|ts| ts.sample.get(bssid))
    }

    /// Get the newest scanner observation retained for a BSSID, even when it
    /// was absent from the latest scan. Stable BSSIDs can remain in the ring
    /// for several samples; background resolution still needs the exact SSID
    /// from their most recent local observation.
    pub fn most_recent_entry(&self, bssid: &str) -> Option<&ScanEntry> {
        self.ring
            .iter()
            .rev()
            .find_map(|timestamped| timestamped.sample.get(bssid))
    }

    /// Count how many of the last N samples contain this BSSID.
    pub fn count(&self, bssid: &str) -> usize {
        self.ring
            .iter()
            .filter(|ts| ts.sample.contains_key(bssid))
            .count()
    }

    /// Get the threshold needed for stability.
    pub fn threshold(&self) -> usize {
        self.threshold
    }

    /// How many samples are currently in the ring buffer.
    pub fn sample_count(&self) -> usize {
        self.ring.len()
    }

    /// Get the most recent scan sample.
    pub fn latest_scan(&self) -> Option<&ScanSample> {
        self.ring.back().map(|ts| &ts.sample)
    }

    /// Get the latest sample and its timestamps/evidence as one atomic ring
    /// entry. Callers holding the Debouncer lock cannot accidentally pair a
    /// newer sample with older refresh evidence.
    pub fn latest_snapshot(&self) -> Option<&TimestampedScan> {
        self.ring.back()
    }

    /// Age of the most recent scan in milliseconds. None if no scans yet.
    pub fn latest_scan_age_ms(&self) -> Option<u64> {
        self.ring
            .back()
            .map(|ts| ts.received_at.elapsed().as_millis() as u64)
    }

    /// Wall-clock UTC time of the most recent scan.
    pub fn latest_scan_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.ring.back().map(|ts| ts.wall_clock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(signal: i32) -> ScanEntry {
        ScanEntry {
            signal_dbm: signal,
            rssi_source: crate::rssi::RssiSource::IwDbm,
            ssid: None,
            channel: None,
        }
    }

    #[test]
    fn test_empty_debouncer() {
        let d = Debouncer::new(10, 5);
        assert!(!d.is_stable("AA:BB:CC:DD:EE:FF"));
        assert!(d.stable_bssids().is_empty());
    }

    #[test]
    fn test_stable_after_threshold() {
        let mut d = Debouncer::new(10, 3);
        let bssid = "AA:BB:CC:DD:EE:FF";

        for _ in 0..2 {
            let mut sample = HashMap::new();
            sample.insert(bssid.to_string(), entry(-65));
            d.push_scan(sample);
        }
        assert!(!d.is_stable(bssid));

        let mut sample = HashMap::new();
        sample.insert(bssid.to_string(), entry(-70));
        d.push_scan(sample);
        assert!(d.is_stable(bssid));
    }

    #[test]
    fn test_window_rollover() {
        let mut d = Debouncer::new(3, 2);
        let bssid = "AA:BB:CC:DD:EE:FF";

        for _ in 0..3 {
            let mut sample = HashMap::new();
            sample.insert(bssid.to_string(), entry(-65));
            d.push_scan(sample);
        }
        assert!(d.is_stable(bssid));

        d.push_scan(HashMap::new());
        d.push_scan(HashMap::new());
        assert!(!d.is_stable(bssid));
    }

    #[test]
    fn clear_forgets_every_sample() {
        let mut d = Debouncer::new(3, 1);
        let mut sample = HashMap::new();
        sample.insert("AA:BB:CC:DD:EE:FF".to_string(), entry(-65));
        d.push_scan(sample);
        assert_eq!(d.sample_count(), 1);

        d.clear();
        assert_eq!(d.sample_count(), 0);
        assert!(d.latest_scan().is_none());
        assert!(d.latest_snapshot().is_none());
        assert!(d.stable_bssids().is_empty());
    }

    #[test]
    fn sample_and_refresh_evidence_are_replaced_and_cleared_atomically() {
        let mut d = Debouncer::new(1, 1);
        let first_evidence = BootTime::from_duration(std::time::Duration::from_secs(10));
        let second_evidence = BootTime::from_duration(std::time::Duration::from_secs(20));

        let mut first = HashMap::new();
        first.insert("AA:BB:CC:DD:EE:01".to_string(), entry(-50));
        d.push_scan_with_refresh_evidence(first, Some(first_evidence));
        assert_eq!(
            d.latest_snapshot().unwrap().confirmed_refresh_started_at,
            Some(first_evidence)
        );

        let mut second = HashMap::new();
        second.insert("AA:BB:CC:DD:EE:02".to_string(), entry(-60));
        d.push_scan_with_refresh_evidence(second, Some(second_evidence));
        let latest = d.latest_snapshot().unwrap();
        assert!(latest.sample.contains_key("AA:BB:CC:DD:EE:02"));
        assert!(!latest.sample.contains_key("AA:BB:CC:DD:EE:01"));
        assert_eq!(latest.confirmed_refresh_started_at, Some(second_evidence));

        d.clear();
        assert!(d.latest_snapshot().is_none());
    }

    #[test]
    fn test_stable_bssids() {
        let mut d = Debouncer::new(5, 2);

        let mut s1 = HashMap::new();
        s1.insert("AA:BB:CC:DD:EE:FF".to_string(), entry(-65));
        s1.insert("11:22:33:44:55:66".to_string(), entry(-80));
        d.push_scan(s1);

        let mut s2 = HashMap::new();
        s2.insert("AA:BB:CC:DD:EE:FF".to_string(), entry(-60));
        d.push_scan(s2);

        let stable = d.stable_bssids();
        assert!(stable.contains("AA:BB:CC:DD:EE:FF"));
        assert!(!stable.contains("11:22:33:44:55:66"));
    }

    #[test]
    fn test_latest_signal() {
        let mut d = Debouncer::new(5, 1);

        let mut s1 = HashMap::new();
        s1.insert("AA:BB:CC:DD:EE:FF".to_string(), entry(-65));
        d.push_scan(s1);

        let mut s2 = HashMap::new();
        s2.insert("AA:BB:CC:DD:EE:FF".to_string(), entry(-50));
        d.push_scan(s2);

        assert_eq!(d.latest_signal("AA:BB:CC:DD:EE:FF"), Some(-50));
    }

    #[test]
    fn test_latest_signal_only_from_latest_scan() {
        let mut d = Debouncer::new(5, 1);

        let mut s1 = HashMap::new();
        s1.insert("AA:BB:CC:DD:EE:FF".to_string(), entry(-65));
        d.push_scan(s1);

        // Second scan does NOT contain the BSSID
        d.push_scan(HashMap::new());

        // Should return None since it's not in the latest scan
        assert_eq!(d.latest_signal("AA:BB:CC:DD:EE:FF"), None);
    }

    #[test]
    fn most_recent_entry_preserves_scanner_ssid_when_latest_scan_misses_bssid() {
        let mut d = Debouncer::new(5, 1);
        let bssid = "AA:BB:CC:DD:EE:FF";
        let mut first = HashMap::new();
        first.insert(
            bssid.to_owned(),
            ScanEntry {
                signal_dbm: -45,
                rssi_source: crate::rssi::RssiSource::IwDbm,
                ssid: Some("MOVIA_BUS_42".to_owned()),
                channel: Some(Channel::TwoFourGhz(6)),
            },
        );
        d.push_scan(first);
        d.push_scan(HashMap::new());

        assert!(d.latest_entry(bssid).is_none());
        assert_eq!(
            d.most_recent_entry(bssid)
                .and_then(|entry| entry.ssid.as_deref()),
            Some("MOVIA_BUS_42")
        );
    }
}
