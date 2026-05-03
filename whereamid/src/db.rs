//! SQLite database: schema creation, WAL mode, CRUD operations for aps/not_found/pending/metadata.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use tracing::{debug, info};

/// Information about a resolved access point.
#[derive(Debug, Clone)]
pub struct ApInfo {
    pub bssid: String,
    pub ssid: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub encryption: Option<String>,
    pub channel: Option<i32>,
    pub frequency: Option<i32>,
    pub city: Option<String>,
    pub country: Option<String>,
    pub source: String,
}

/// A pending BSSID awaiting resolution.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PendingAp {
    pub bssid: String,
    pub ssid: Option<String>,
    pub channel: Option<i32>,
    pub frequency: Option<i32>,
    pub signal_dbm: Option<i32>,
    pub attempts: i32,
}

pub struct Database {
    conn: Connection,
}

const SCHEMA_VERSION: i32 = 1;

impl Database {
    /// Open or create the database at `path`. Sets WAL mode and creates schema if needed.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database at {}", path.display()))?;

        let wal_mode: String = conn
            .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
            .context("failed to set WAL mode")?;
        if wal_mode.to_lowercase() != "wal" {
            tracing::warn!(
                "WAL mode requested but got '{}' — performance may be degraded",
                wal_mode
            );
        }
        conn.pragma_update(None, "foreign_keys", "ON")
            .context("failed to enable foreign keys")?;

        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Open an in-memory database (for testing).
    #[allow(dead_code)]
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("failed to open in-memory database")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let version = self.get_schema_version();

        if version == 0 {
            info!("initializing database schema v{SCHEMA_VERSION}");
            self.create_schema_v1()?;
        } else if version < SCHEMA_VERSION {
            info!("migrating database from v{version} to v{SCHEMA_VERSION}");
            // Future migrations go here
        } else {
            debug!("database schema is up to date (v{version})");
        }

        Ok(())
    }

    fn get_schema_version(&self) -> i32 {
        // Table may not exist yet
        let result: Result<i32, _> =
            self.conn
                .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                    row.get(0)
                });
        result.unwrap_or(0)
    }

    fn create_schema_v1(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS aps (
                bssid       TEXT PRIMARY KEY,
                ssid        TEXT,
                lat         REAL NOT NULL,
                lon         REAL NOT NULL,
                encryption  TEXT,
                channel     INTEGER,
                frequency   INTEGER,
                city        TEXT,
                country     TEXT,
                source      TEXT NOT NULL,
                first_seen  TEXT NOT NULL,
                last_seen   TEXT NOT NULL,
                fetched_at  TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_aps_geo ON aps(lat, lon);
            CREATE INDEX IF NOT EXISTS idx_aps_last_seen ON aps(last_seen);

            CREATE TABLE IF NOT EXISTS not_found (
                bssid       TEXT PRIMARY KEY,
                first_seen  TEXT NOT NULL,
                last_seen   TEXT NOT NULL,
                checked_at  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pending (
                bssid       TEXT PRIMARY KEY,
                ssid        TEXT,
                channel     INTEGER,
                frequency   INTEGER,
                signal_dbm  INTEGER,
                first_seen  TEXT NOT NULL,
                last_seen   TEXT NOT NULL,
                attempts    INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS metadata (
                key         TEXT PRIMARY KEY,
                value       TEXT NOT NULL
            );
        ",
            )
            .context("failed to create schema v1")?;

        // Set version, but only if not already set
        let count: i32 = self
            .conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))?;
        if count == 0 {
            self.conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![SCHEMA_VERSION],
            )?;
        }

        Ok(())
    }

    // --- aps table ---

    /// Look up an AP by BSSID. Returns None if not cached.
    pub fn get_ap(&self, bssid: &str) -> Result<Option<ApInfo>> {
        let result = self
            .conn
            .query_row(
                "SELECT bssid, ssid, lat, lon, encryption, channel, frequency, city, country, source
                 FROM aps WHERE bssid = ?1",
                params![bssid],
                |row| {
                    Ok(ApInfo {
                        bssid: row.get(0)?,
                        ssid: row.get(1)?,
                        lat: row.get(2)?,
                        lon: row.get(3)?,
                        encryption: row.get(4)?,
                        channel: row.get(5)?,
                        frequency: row.get(6)?,
                        city: row.get(7)?,
                        country: row.get(8)?,
                        source: row.get(9)?,
                    })
                },
            )
            .optional()?;
        Ok(result)
    }

    /// Insert or update a resolved AP.
    pub fn upsert_ap(&self, ap: &ApInfo) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO aps (bssid, ssid, lat, lon, encryption, channel, frequency, city, country, source, first_seen, last_seen, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?11)
             ON CONFLICT(bssid) DO UPDATE SET
                ssid = excluded.ssid,
                lat = excluded.lat,
                lon = excluded.lon,
                encryption = excluded.encryption,
                channel = excluded.channel,
                frequency = excluded.frequency,
                city = excluded.city,
                country = excluded.country,
                source = excluded.source,
                last_seen = excluded.last_seen,
                fetched_at = excluded.fetched_at",
            params![ap.bssid, ap.ssid, ap.lat, ap.lon, ap.encryption, ap.channel, ap.frequency, ap.city, ap.country, ap.source, now],
        )?;
        Ok(())
    }

    /// Update last_seen for an AP already in the cache.
    pub fn touch_ap(&self, bssid: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE aps SET last_seen = ?1 WHERE bssid = ?2",
            params![now, bssid],
        )?;
        Ok(())
    }

    // --- not_found table ---

    /// Check if a BSSID is in the not_found table and not expired.
    pub fn is_not_found(&self, bssid: &str, ttl_days: i64) -> Result<bool> {
        let cutoff = (Utc::now() - chrono::TimeDelta::days(ttl_days)).to_rfc3339();
        let count: i32 = self.conn.query_row(
            "SELECT COUNT(*) FROM not_found WHERE bssid = ?1 AND checked_at > ?2",
            params![bssid, cutoff],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Insert into not_found.
    pub fn insert_not_found(&self, bssid: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO not_found (bssid, first_seen, last_seen, checked_at)
             VALUES (?1, ?2, ?2, ?2)
             ON CONFLICT(bssid) DO UPDATE SET last_seen = excluded.last_seen, checked_at = excluded.checked_at",
            params![bssid, now],
        )?;
        Ok(())
    }

    // --- pending table ---

    /// Insert a BSSID into the pending queue.
    pub fn insert_pending(
        &self,
        bssid: &str,
        ssid: Option<&str>,
        channel: Option<i32>,
        frequency: Option<i32>,
        signal_dbm: Option<i32>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO pending (bssid, ssid, channel, frequency, signal_dbm, first_seen, last_seen, attempts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 0)
             ON CONFLICT(bssid) DO UPDATE SET
                ssid = COALESCE(excluded.ssid, pending.ssid),
                last_seen = excluded.last_seen,
                signal_dbm = CASE WHEN excluded.signal_dbm > pending.signal_dbm THEN excluded.signal_dbm ELSE pending.signal_dbm END",
            params![bssid, ssid, channel, frequency, signal_dbm, now],
        )?;
        Ok(())
    }

    /// Get up to `limit` pending BSSIDs, ordered by fewest attempts first.
    pub fn get_pending(&self, limit: usize) -> Result<Vec<PendingAp>> {
        let mut stmt = self.conn.prepare(
            "SELECT bssid, ssid, channel, frequency, signal_dbm, attempts
             FROM pending ORDER BY attempts ASC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(PendingAp {
                bssid: row.get(0)?,
                ssid: row.get(1)?,
                channel: row.get(2)?,
                frequency: row.get(3)?,
                signal_dbm: row.get(4)?,
                attempts: row.get(5)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Increment attempts for a pending BSSID.
    pub fn increment_pending_attempts(&self, bssid: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE pending SET attempts = attempts + 1 WHERE bssid = ?1",
            params![bssid],
        )?;
        Ok(())
    }

    /// Delete a BSSID from the pending table.
    pub fn delete_pending(&self, bssid: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM pending WHERE bssid = ?1", params![bssid])?;
        Ok(())
    }

    /// Delete pending entries that have exceeded max attempts.
    pub fn delete_expired_pending(&self, max_attempts: i32) -> Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM pending WHERE attempts >= ?1",
            params![max_attempts],
        )?;
        Ok(deleted)
    }

    // --- metadata (rate limiting) ---

    /// Get a metadata value by key.
    pub fn get_metadata(&self, key: &str) -> Result<Option<String>> {
        let result = self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(result)
    }

    /// Set a metadata value.
    pub fn set_metadata(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Check if we can make an API call today (under daily limit).
    pub fn can_call_api(&self, daily_limit: u32) -> Result<bool> {
        self.maybe_reset_daily_counter()?;
        let count = self.api_calls_today()?;
        Ok(count < daily_limit)
    }

    /// Record an API call. Increments the daily counter.
    pub fn record_api_call(&self) -> Result<()> {
        self.maybe_reset_daily_counter()?;
        let count = self.api_calls_today()?;
        self.set_metadata("api_calls_today", &(count + 1).to_string())?;
        Ok(())
    }

    /// Get the number of API calls made today.
    pub fn api_calls_today(&self) -> Result<u32> {
        match self.get_metadata("api_calls_today")? {
            Some(v) => Ok(v.parse().unwrap_or(0)),
            None => Ok(0),
        }
    }

    fn maybe_reset_daily_counter(&self) -> Result<()> {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let stored_date = self.get_metadata("api_calls_date")?;
        if stored_date.as_deref() != Some(&today) {
            self.set_metadata("api_calls_date", &today)?;
            self.set_metadata("api_calls_today", "0")?;
        }
        Ok(())
    }

    // --- stats ---

    pub fn cached_ap_count(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM aps", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn pending_ap_count(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM pending", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn not_found_ap_count(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM not_found", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn db_size_bytes(&self) -> Result<i64> {
        let page_count: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let page_size: i64 = self
            .conn
            .query_row("PRAGMA page_size", [], |row| row.get(0))?;
        Ok(page_count * page_size)
    }

    /// Get expired not_found entries (older than ttl_days) for re-checking.
    pub fn get_expired_not_found(&self, ttl_days: i64, limit: usize) -> Result<Vec<String>> {
        let cutoff = (Utc::now() - chrono::TimeDelta::days(ttl_days)).to_rfc3339();
        let mut stmt = self
            .conn
            .prepare("SELECT bssid FROM not_found WHERE checked_at <= ?1 LIMIT ?2")?;
        let rows = stmt.query_map(params![cutoff, limit as i64], |row| row.get(0))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Delete a BSSID from not_found (e.g., for re-checking).
    pub fn delete_not_found(&self, bssid: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM not_found WHERE bssid = ?1", params![bssid])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_creation() {
        let db = Database::open_memory().unwrap();
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn test_ap_crud() {
        let db = Database::open_memory().unwrap();
        let ap = ApInfo {
            bssid: "AA:BB:CC:DD:EE:FF".to_string(),
            ssid: Some("TestWiFi".to_string()),
            lat: 55.6684,
            lon: 12.5541,
            encryption: None,
            channel: Some(6),
            frequency: Some(2437),
            city: None,
            country: None,
            source: "wigle".to_string(),
        };
        db.upsert_ap(&ap).unwrap();
        let got = db.get_ap("AA:BB:CC:DD:EE:FF").unwrap().unwrap();
        assert_eq!(got.ssid, Some("TestWiFi".to_string()));
        assert!((got.lat - 55.6684).abs() < 1e-6);
    }

    #[test]
    fn test_not_found() {
        let db = Database::open_memory().unwrap();
        db.insert_not_found("AA:BB:CC:DD:EE:FF").unwrap();
        assert!(db.is_not_found("AA:BB:CC:DD:EE:FF", 30).unwrap());
        assert!(!db.is_not_found("11:22:33:44:55:66", 30).unwrap());
    }

    #[test]
    fn test_pending() {
        let db = Database::open_memory().unwrap();
        db.insert_pending(
            "AA:BB:CC:DD:EE:FF",
            Some("Test"),
            Some(6),
            Some(2437),
            Some(-65),
        )
        .unwrap();
        let pending = db.get_pending(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].bssid, "AA:BB:CC:DD:EE:FF");
        assert_eq!(pending[0].attempts, 0);

        db.increment_pending_attempts("AA:BB:CC:DD:EE:FF").unwrap();
        let pending = db.get_pending(10).unwrap();
        assert_eq!(pending[0].attempts, 1);

        db.delete_pending("AA:BB:CC:DD:EE:FF").unwrap();
        let pending = db.get_pending(10).unwrap();
        assert!(pending.is_empty());
    }

    #[test]
    fn test_rate_limiting() {
        let db = Database::open_memory().unwrap();
        assert!(db.can_call_api(100).unwrap());
        assert_eq!(db.api_calls_today().unwrap(), 0);

        db.record_api_call().unwrap();
        assert_eq!(db.api_calls_today().unwrap(), 1);

        assert!(!db.can_call_api(1).unwrap());
        assert!(db.can_call_api(2).unwrap());
    }
}
