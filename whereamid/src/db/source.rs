//! Origin of an AP fix, with an explicit priority ranking.
//!
//! We persist (bssid, source, source_priority) so reads always return the
//! highest-quality known fix and lower-quality writes cannot overwrite it.
//! Higher numeric value = higher priority.
//!
//! The ordering reflects empirical accuracy:
//! - Apple WPS aggregates massive crowdsourced telemetry from iOS devices and
//!   is generally tightest.
//! - WiGLE is community-submitted wardriving data; coverage is great but the
//!   per-AP position can drift.
//! - BeaconDB is similar to WiGLE in spirit but smaller. Source::BeaconDb is
//!   read-only legacy: no production path writes it today (BeaconDbClient was
//!   removed in task-0034) but historical DB rows still read back at this
//!   priority.
//! - Manual is a user-supplied override with no implicit trust ordering;
//!   placed lowest deliberately so the system can correct user mistakes by
//!   preferring authoritative sources.
//!
//! task-0055 split this enum out of db.rs into its own submodule. Public
//! API surface unchanged; `crate::db::Source` remains the canonical path.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Apple,
    Wigle,
    /// Read-only legacy. No production path writes this today; the variant
    /// exists so historical DB rows from earlier prototypes still read back
    /// at the correct priority.
    BeaconDb,
    Manual,
    /// Anything we read back from the DB that we no longer recognise.
    /// Treated as the lowest priority so a real source can always win.
    Unknown,
}

impl Source {
    /// Numeric priority. Higher = more trusted.
    pub fn priority(self) -> i32 {
        match self {
            Source::Apple => 40,
            Source::Wigle => 30,
            Source::BeaconDb => 20,
            Source::Manual => 10,
            Source::Unknown => 0,
        }
    }

    /// Canonical wire-format string stored in `aps.source`.
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Apple => "apple",
            Source::Wigle => "wigle",
            Source::BeaconDb => "beacondb",
            Source::Manual => "manual",
            Source::Unknown => "unknown",
        }
    }

    /// Parse a stored `source` string. Unknown values map to `Source::Unknown`
    /// (lowest priority) so they cannot win against any recognised source.
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "apple" => Source::Apple,
            "wigle" => Source::Wigle,
            "beacondb" => Source::BeaconDb,
            "manual" => Source::Manual,
            _ => Source::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Source priority enum is internally consistent: ordering is strict
    /// and matches what the migration backfill SQL hardcodes.
    #[test]
    fn source_priority_ladder() {
        assert!(Source::Apple.priority() > Source::Wigle.priority());
        assert!(Source::Wigle.priority() > Source::BeaconDb.priority());
        assert!(Source::BeaconDb.priority() > Source::Manual.priority());
        assert!(Source::Manual.priority() > Source::Unknown.priority());

        for s in [
            Source::Apple,
            Source::Wigle,
            Source::BeaconDb,
            Source::Manual,
            Source::Unknown,
        ] {
            assert_eq!(Source::from_db_str(s.as_str()), s);
        }
        assert_eq!(Source::from_db_str("garbage"), Source::Unknown);
        assert_eq!(Source::from_db_str(""), Source::Unknown);
    }
}
