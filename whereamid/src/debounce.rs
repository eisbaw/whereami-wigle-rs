//! Debounce: sliding window stable AP detection.
//!
//! Maintains a ring buffer of the last N scan samples. A BSSID is "stable"
//! when it appears in >= M of those N samples.

use std::collections::{HashMap, HashSet, VecDeque};

/// Per-AP info stored in each scan sample.
#[derive(Debug, Clone)]
pub struct ScanEntry {
    pub signal_dbm: i32,
    pub ssid: Option<String>,
    pub channel: Option<i32>,
}

/// A single scan sample: maps BSSID -> scan entry.
pub type ScanSample = HashMap<String, ScanEntry>;

/// Debounce ring buffer for stable AP detection.
pub struct Debouncer {
    ring: VecDeque<ScanSample>,
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
    pub fn push_scan(&mut self, sample: ScanSample) {
        if self.ring.len() >= self.window {
            self.ring.pop_front();
        }
        self.ring.push_back(sample);
    }

    /// Check if a specific BSSID is stable (appears in >= threshold samples).
    #[allow(dead_code)]
    pub fn is_stable(&self, bssid: &str) -> bool {
        let count = self.ring.iter().filter(|s| s.contains_key(bssid)).count();
        count >= self.threshold
    }

    /// Return all currently stable BSSIDs.
    pub fn stable_bssids(&self) -> HashSet<String> {
        // Count appearances of each BSSID across all samples
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for sample in &self.ring {
            for bssid in sample.keys() {
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
    pub fn latest_signal(&self, bssid: &str) -> Option<i32> {
        self.ring
            .back()
            .and_then(|sample| sample.get(bssid).map(|e| e.signal_dbm))
    }

    /// Get the full scan entry for a BSSID from the most recent scan.
    #[allow(dead_code)]
    pub fn latest_entry(&self, bssid: &str) -> Option<&ScanEntry> {
        self.ring.back().and_then(|sample| sample.get(bssid))
    }

    /// How many samples are currently in the ring buffer.
    pub fn sample_count(&self) -> usize {
        self.ring.len()
    }

    /// Get the most recent scan sample.
    pub fn latest_scan(&self) -> Option<&ScanSample> {
        self.ring.back()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(signal: i32) -> ScanEntry {
        ScanEntry {
            signal_dbm: signal,
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
}
