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

/// Origin of an AP fix, with an explicit ranking.
///
/// We persist (bssid, source, source_priority) so reads always return the
/// highest-quality known fix and lower-quality writes cannot overwrite it.
/// Higher numeric value = higher priority.
///
/// The ordering reflects empirical accuracy:
/// - Apple WPS aggregates massive crowdsourced telemetry from iOS devices and
///   is generally tightest.
/// - WiGLE is community-submitted wardriving data; coverage is great but the
///   per-AP position can drift.
/// - BeaconDB is similar to WiGLE in spirit but smaller.
/// - Manual is a user-supplied override with no implicit trust ordering;
///   placed lowest deliberately so the system can correct user mistakes by
///   preferring authoritative sources. If callers want a sticky manual
///   override they should bump priority explicitly. (TODO if that becomes a
///   real requirement.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Apple,
    Wigle,
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

const SCHEMA_VERSION: i32 = 4;

/// A persisted last-known position. Stored as a single row in `last_fix`
/// (CHECK id = 1) so the daemon can rehydrate after a restart.
#[derive(Debug, Clone)]
pub struct LastFixRow {
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: f64,
    pub address: Option<String>,
    /// RFC3339 timestamp; the in-memory representation in server.rs uses
    /// chrono::DateTime<Utc> for age computation across restarts.
    pub at_rfc3339: String,
    pub sources: i64,
}

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
        let mut version = self.get_schema_version();

        if version == 0 {
            info!("initializing database schema v{SCHEMA_VERSION}");
            self.create_schema_v1()?;
            version = 1;
        }

        if version < 2 {
            info!("migrating database v{version} -> v2 (add aps.source_priority)");
            self.migrate_v1_to_v2()?;
            version = 2;
        }

        if version < 3 {
            info!("migrating database v{version} -> v3 (single-row schema_version)");
            self.migrate_v2_to_v3()?;
            version = 3;
        }

        if version < 4 {
            info!("migrating database v{version} -> v4 (last_fix table)");
            self.migrate_v3_to_v4()?;
            version = 4;
        }

        if version == SCHEMA_VERSION {
            debug!("database schema is up to date (v{version})");
        } else if version > SCHEMA_VERSION {
            tracing::warn!(
                "database schema is v{version} but binary expects v{SCHEMA_VERSION}; \
                 forward compatibility is not guaranteed"
            );
        }

        Ok(())
    }

    /// v1 -> v2: add `aps.source_priority` and backfill from the existing
    /// `source` column. Idempotent: safe to run multiple times because the
    /// schema_version row is the gating condition; we never reach this from
    /// a v2 database.
    fn migrate_v1_to_v2(&self) -> Result<()> {
        // Defensive: if the column already exists (e.g. partial earlier run),
        // skip the ALTER. We detect that via PRAGMA table_info.
        let has_priority: bool = {
            let mut stmt = self.conn.prepare("PRAGMA table_info(aps)")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
            let mut found = false;
            for row in rows {
                if row? == "source_priority" {
                    found = true;
                    break;
                }
            }
            found
        };
        if !has_priority {
            self.conn
                .execute(
                    "ALTER TABLE aps ADD COLUMN source_priority INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .context("failed to add aps.source_priority column")?;
        }

        // Backfill priorities from existing source strings. Hardcode the
        // numbers here rather than calling Source::priority(): future code
        // changes to the enum must not silently rewrite historical data.
        // (If you change the priority ladder, write a new migration.)
        self.conn.execute_batch(
            "UPDATE aps SET source_priority = 40 WHERE source = 'apple';
             UPDATE aps SET source_priority = 30 WHERE source = 'wigle';
             UPDATE aps SET source_priority = 20 WHERE source = 'beacondb';
             UPDATE aps SET source_priority = 10 WHERE source = 'manual';
             UPDATE aps SET source_priority = 0  WHERE source NOT IN ('apple','wigle','beacondb','manual');",
        )?;

        // Bump schema_version. The row-management is conservative: update if
        // a row exists, otherwise insert. (Task 0030 will tighten this.)
        let updated = self
            .conn
            .execute("UPDATE schema_version SET version = 2", [])?;
        if updated == 0 {
            self.conn
                .execute("INSERT INTO schema_version (version) VALUES (2)", [])?;
        }
        Ok(())
    }

    /// v2 -> v3: rebuild `schema_version` so it can hold at most one row,
    /// keyed by `id = 1`. SQLite cannot ALTER TABLE to add a CHECK
    /// constraint, so we have to recreate the table.
    ///
    /// Behaviour for malformed v2 databases (multiple rows): coalesce to
    /// MAX(version) — pick the highest version any row claims and discard
    /// the others. This is the conservative choice; a smaller value could
    /// trick `migrate()` into re-running an already-applied migration.
    ///
    /// Idempotent guard: detect if the new shape is already present
    /// (someone might run migrate twice through different code paths) and
    /// skip the rebuild in that case.
    fn migrate_v2_to_v3(&self) -> Result<()> {
        // Detect if the table is already single-row constrained. We look at
        // the SQL stored in sqlite_master for the schema_version table.
        let sql_present: Option<String> = self
            .conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let already_constrained = sql_present
            .as_deref()
            .map(|s| s.contains("CHECK (id = 1)"))
            .unwrap_or(false);
        if already_constrained {
            // Still bump version-row content to SCHEMA_VERSION below if needed.
            let updated = self.conn.execute(
                "UPDATE schema_version SET version = ?1 WHERE id = 1",
                params![SCHEMA_VERSION],
            )?;
            if updated == 0 {
                self.conn.execute(
                    "INSERT INTO schema_version (id, version) VALUES (1, ?1)",
                    params![SCHEMA_VERSION],
                )?;
            }
            return Ok(());
        }

        // Rebuild atomically. If anything fails the original table survives.
        let tx = self
            .conn
            .unchecked_transaction()
            .context("failed to start transaction for schema_version rebuild")?;
        tx.execute_batch(
            "CREATE TABLE schema_version_new (
                 id      INTEGER PRIMARY KEY CHECK (id = 1),
                 version INTEGER NOT NULL
             );
             INSERT INTO schema_version_new (id, version)
                 VALUES (1, COALESCE((SELECT MAX(version) FROM schema_version), 0));
             DROP TABLE schema_version;
             ALTER TABLE schema_version_new RENAME TO schema_version;",
        )
        .context("failed to rebuild schema_version with single-row constraint")?;

        // Stamp the (now single) row to SCHEMA_VERSION. The version field
        // could already be >= 3 if this migration was somehow applied out of
        // order; in that case keep the higher value.
        tx.execute(
            "UPDATE schema_version SET version = ?1 WHERE id = 1 AND version < ?1",
            params![SCHEMA_VERSION],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// v3 -> v4: add the `last_fix` table with a single-row CHECK so the
    /// daemon can persist its last-known position across restarts. Idempotent
    /// via `CREATE TABLE IF NOT EXISTS` and a fresh schema_version stamp.
    fn migrate_v3_to_v4(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS last_fix (
                     id          INTEGER PRIMARY KEY CHECK (id = 1),
                     lat         REAL NOT NULL,
                     lon         REAL NOT NULL,
                     accuracy_m  REAL NOT NULL,
                     address     TEXT,
                     at_rfc3339  TEXT NOT NULL,
                     sources     INTEGER NOT NULL
                 );",
            )
            .context("failed to create last_fix table")?;
        self.conn.execute(
            "UPDATE schema_version SET version = ?1 WHERE id = 1",
            params![SCHEMA_VERSION],
        )?;
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

        // Set version to 1 (this function only creates the v1 shape).
        // Subsequent migrate_v1_to_v2 / migrate_v2_to_v3 will bump it.
        // We previously inserted SCHEMA_VERSION here, which short-circuited
        // newer migrations on a fresh database.
        let count: i32 = self
            .conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))?;
        if count == 0 {
            self.conn
                .execute("INSERT INTO schema_version (version) VALUES (1)", [])?;
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
    ///
    /// Source priority is enforced: an existing row is overwritten only if
    /// the incoming row's source priority is **strictly greater than or
    /// equal to** the stored row's. This keeps better fixes (Apple) sticky
    /// against later, lower-quality writes (WiGLE/BeaconDB) for the same
    /// BSSID.
    ///
    /// `last_seen` is **always** advanced regardless of priority decision —
    /// observing a BSSID is independent of whose data is authoritative.
    pub fn upsert_ap(&self, ap: &ApInfo) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let priority = Source::from_db_str(&ap.source).priority();
        self.conn.execute(
            "INSERT INTO aps (bssid, ssid, lat, lon, encryption, channel, frequency, city, country, source, source_priority, first_seen, last_seen, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, ?12)
             ON CONFLICT(bssid) DO UPDATE SET
                ssid       = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.ssid       ELSE aps.ssid       END,
                lat        = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.lat        ELSE aps.lat        END,
                lon        = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.lon        ELSE aps.lon        END,
                encryption = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.encryption ELSE aps.encryption END,
                channel    = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.channel    ELSE aps.channel    END,
                frequency  = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.frequency  ELSE aps.frequency  END,
                city       = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.city       ELSE aps.city       END,
                country    = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.country    ELSE aps.country    END,
                source           = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.source           ELSE aps.source           END,
                source_priority  = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.source_priority  ELSE aps.source_priority  END,
                fetched_at       = CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.fetched_at       ELSE aps.fetched_at       END,
                last_seen  = excluded.last_seen",
            params![ap.bssid, ap.ssid, ap.lat, ap.lon, ap.encryption, ap.channel, ap.frequency, ap.city, ap.country, ap.source, priority, now],
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

    /// Cheap existence check: is this BSSID currently in the pending queue?
    ///
    /// Uses an indexed PRIMARY KEY probe. Prefer this over
    /// `get_pending(N).iter().any(...)` — that pattern is O(N) per call and
    /// scans rows we never inspect. For per-BSSID checks (debug rendering,
    /// not_found end-of-chain logic) this is the right primitive.
    pub fn is_pending(&self, bssid: &str) -> Result<bool> {
        let exists: Option<i32> = self
            .conn
            .query_row(
                "SELECT 1 FROM pending WHERE bssid = ?1 LIMIT 1",
                params![bssid],
                |row| row.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
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

    /// Atomically reserve an API call slot for today.
    ///
    /// Performs daily-counter reset, current-count read, and increment in a
    /// single IMMEDIATE transaction. Returns `Ok(true)` if a slot was reserved
    /// (caller may proceed with the network call) or `Ok(false)` if the
    /// daily limit is already exhausted.
    ///
    /// On any transient failure of the network call afterwards, the caller
    /// SHOULD call [`Database::refund_api_call`] to give the slot back.
    /// Slots that map to actual completed lookups (Found / NotFound) should
    /// not be refunded.
    pub fn try_reserve_api_call(&mut self, daily_limit: u32) -> Result<bool> {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let tx = self.conn.transaction()?;

        // Reset counter if the stored date is not today.
        let stored_date: Option<String> = tx
            .query_row(
                "SELECT value FROM metadata WHERE key = 'api_calls_date'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let count: u32 = if stored_date.as_deref() == Some(&today) {
            tx.query_row(
                "SELECT value FROM metadata WHERE key = 'api_calls_today'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
        } else {
            tx.execute(
                "INSERT INTO metadata (key, value) VALUES ('api_calls_date', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![&today],
            )?;
            0
        };

        if count >= daily_limit {
            // Persist the date reset (if any) without bumping the counter.
            tx.execute(
                "INSERT INTO metadata (key, value) VALUES ('api_calls_today', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![count.to_string()],
            )?;
            tx.commit()?;
            return Ok(false);
        }

        let new_count = count + 1;
        tx.execute(
            "INSERT INTO metadata (key, value) VALUES ('api_calls_today', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![new_count.to_string()],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Refund a previously reserved API slot. Clamped at 0. Only meaningful
    /// when called on the same calendar day; if the day has rolled over the
    /// refund is silently ignored (counter has already been reset to 0).
    pub fn refund_api_call(&mut self) -> Result<()> {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let tx = self.conn.transaction()?;
        let stored_date: Option<String> = tx
            .query_row(
                "SELECT value FROM metadata WHERE key = 'api_calls_date'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if stored_date.as_deref() != Some(&today) {
            // Day rolled over since reservation; counter already reset.
            tx.commit()?;
            return Ok(());
        }
        let count: u32 = tx
            .query_row(
                "SELECT value FROM metadata WHERE key = 'api_calls_today'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let new_count = count.saturating_sub(1);
        tx.execute(
            "INSERT INTO metadata (key, value) VALUES ('api_calls_today', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![new_count.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Get the number of API calls made today.
    pub fn api_calls_today(&self) -> Result<u32> {
        match self.get_metadata("api_calls_today")? {
            Some(v) => Ok(v.parse().unwrap_or(0)),
            None => Ok(0),
        }
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

    // --- last_fix table ---

    /// Persist the last-known position. Single-row table; an existing row
    /// is overwritten unconditionally.
    pub fn set_last_fix(&self, fix: &LastFixRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO last_fix (id, lat, lon, accuracy_m, address, at_rfc3339, sources)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                lat        = excluded.lat,
                lon        = excluded.lon,
                accuracy_m = excluded.accuracy_m,
                address    = excluded.address,
                at_rfc3339 = excluded.at_rfc3339,
                sources    = excluded.sources",
            params![
                fix.lat,
                fix.lon,
                fix.accuracy_m,
                fix.address,
                fix.at_rfc3339,
                fix.sources
            ],
        )?;
        Ok(())
    }

    /// Read the persisted last-known position, if any.
    pub fn get_last_fix(&self) -> Result<Option<LastFixRow>> {
        let row = self
            .conn
            .query_row(
                "SELECT lat, lon, accuracy_m, address, at_rfc3339, sources
                 FROM last_fix WHERE id = 1",
                [],
                |row| {
                    Ok(LastFixRow {
                        lat: row.get(0)?,
                        lon: row.get(1)?,
                        accuracy_m: row.get(2)?,
                        address: row.get(3)?,
                        at_rfc3339: row.get(4)?,
                        sources: row.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
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

    /// Helper: how many rows exist in schema_version, regardless of shape.
    fn schema_version_row_count(db: &Database) -> i64 {
        db.conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .unwrap()
    }

    /// migrate() must be idempotent: calling it twice on a fresh in-memory
    /// DB must leave exactly one row in schema_version, with version =
    /// SCHEMA_VERSION. This is the headline invariant from task 0030.
    #[test]
    fn migrate_is_idempotent_and_leaves_single_row() {
        let db = Database::open_memory().unwrap();
        // open_memory() already calls migrate() once. Call it again.
        db.migrate().unwrap();

        assert_eq!(
            schema_version_row_count(&db),
            1,
            "schema_version must contain exactly one row"
        );
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);

        // And a third time, just to be paranoid.
        db.migrate().unwrap();
        assert_eq!(schema_version_row_count(&db), 1);
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);
    }

    /// The new schema_version table has a CHECK (id = 1) constraint.
    /// Attempting to insert a second row must fail.
    #[test]
    fn schema_version_rejects_second_row() {
        let db = Database::open_memory().unwrap();
        let err = db
            .conn
            .execute(
                "INSERT INTO schema_version (id, version) VALUES (2, 99)",
                [],
            )
            .unwrap_err();
        let msg = format!("{err}");
        // Either CHECK constraint or PRIMARY KEY uniqueness on a duplicate
        // value would qualify. id=2 should fail the CHECK.
        assert!(
            msg.contains("CHECK") || msg.contains("constraint") || msg.contains("UNIQUE"),
            "expected constraint violation, got: {msg}"
        );
    }

    /// A malformed v1/v2 database with multiple rows in schema_version must
    /// be coalesced to a single row at MAX(version) by the v2->v3 migration.
    #[test]
    fn migrate_collapses_malformed_multirow_schema_version() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version VALUES (1);
             INSERT INTO schema_version VALUES (2);
             INSERT INTO schema_version VALUES (1);
             CREATE TABLE aps (
                 bssid TEXT PRIMARY KEY, ssid TEXT, lat REAL NOT NULL, lon REAL NOT NULL,
                 encryption TEXT, channel INTEGER, frequency INTEGER, city TEXT, country TEXT,
                 source TEXT NOT NULL,
                 source_priority INTEGER NOT NULL DEFAULT 0,
                 first_seen TEXT NOT NULL, last_seen TEXT NOT NULL, fetched_at TEXT NOT NULL
             );
             CREATE TABLE not_found (bssid TEXT PRIMARY KEY, first_seen TEXT, last_seen TEXT, checked_at TEXT);
             CREATE TABLE pending (bssid TEXT PRIMARY KEY, ssid TEXT, channel INTEGER, frequency INTEGER, signal_dbm INTEGER, first_seen TEXT, last_seen TEXT, attempts INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        let db = Database { conn };
        // Looks like v2 to get_schema_version() because LIMIT 1 returns one
        // of the rows; we don't care which. The point is migrate() must
        // collapse to a single row regardless.
        db.migrate().unwrap();

        assert_eq!(schema_version_row_count(&db), 1);
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);

        // Constraint must now be active.
        let err = db
            .conn
            .execute(
                "INSERT INTO schema_version (id, version) VALUES (2, 99)",
                [],
            )
            .unwrap_err();
        assert!(
            format!("{err}").contains("CHECK")
                || format!("{err}").contains("constraint")
                || format!("{err}").contains("UNIQUE"),
            "expected constraint violation, got: {err}"
        );
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

    /// Build an ApInfo with the given source string and lat (used to tell
    /// rows apart in priority assertions).
    fn ap_with(source: &str, lat: f64) -> ApInfo {
        ApInfo {
            bssid: "AA:BB:CC:DD:EE:FF".to_string(),
            ssid: Some(format!("ssid-{source}")),
            lat,
            lon: 12.5541,
            encryption: None,
            channel: Some(6),
            frequency: Some(2437),
            city: None,
            country: None,
            source: source.to_string(),
        }
    }

    /// Source priority enum is internally consistent: ordering is strict
    /// and matches what the migration backfill SQL hardcodes.
    #[test]
    fn source_priority_ladder() {
        assert!(Source::Apple.priority() > Source::Wigle.priority());
        assert!(Source::Wigle.priority() > Source::BeaconDb.priority());
        assert!(Source::BeaconDb.priority() > Source::Manual.priority());
        assert!(Source::Manual.priority() > Source::Unknown.priority());

        // round-trip with the canonical string
        for s in [
            Source::Apple,
            Source::Wigle,
            Source::BeaconDb,
            Source::Manual,
            Source::Unknown,
        ] {
            assert_eq!(Source::from_db_str(s.as_str()), s);
        }
        // unknown strings collapse to Unknown
        assert_eq!(Source::from_db_str("garbage"), Source::Unknown);
        assert_eq!(Source::from_db_str(""), Source::Unknown);
    }

    /// upsert_ap honours the priority ladder. Exercise every cross-source
    /// transition: equal-priority overwrites, higher-priority overwrites,
    /// lower-priority does NOT overwrite the data fields, but `last_seen`
    /// always advances (an observation is always recorded).
    #[test]
    fn upsert_ap_respects_source_priority() {
        let db = Database::open_memory().unwrap();
        let bssid = "AA:BB:CC:DD:EE:FF";

        // 1. Wigle writes first.
        db.upsert_ap(&ap_with("wigle", 1.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, "wigle");
        assert!((got.lat - 1.0).abs() < 1e-9);

        // 2. Lower-priority manual write must NOT overwrite.
        db.upsert_ap(&ap_with("manual", 2.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, "wigle", "manual must not overwrite wigle");
        assert!((got.lat - 1.0).abs() < 1e-9, "lat must not change");

        // 3. Lower-priority beacondb write must NOT overwrite wigle either.
        db.upsert_ap(&ap_with("beacondb", 3.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, "wigle");

        // 4. Equal-priority same-source rewrite SHOULD update the position
        //    (we trust the more recent observation from the same source).
        db.upsert_ap(&ap_with("wigle", 4.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, "wigle");
        assert!(
            (got.lat - 4.0).abs() < 1e-9,
            "same-source rewrite should update lat, got {}",
            got.lat
        );

        // 5. Higher-priority apple write SHOULD overwrite.
        db.upsert_ap(&ap_with("apple", 5.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, "apple");
        assert!((got.lat - 5.0).abs() < 1e-9);

        // 6. Subsequent lower-priority writes (wigle, beacondb, manual,
        //    unknown) must NOT overwrite apple.
        for (src, lat) in [
            ("wigle", 6.0),
            ("beacondb", 7.0),
            ("manual", 8.0),
            ("garbage", 9.0),
        ] {
            db.upsert_ap(&ap_with(src, lat)).unwrap();
            let got = db.get_ap(bssid).unwrap().unwrap();
            assert_eq!(got.source, "apple", "{src} must not overwrite apple");
            assert!((got.lat - 5.0).abs() < 1e-9);
        }
    }

    /// Migration v1 -> v2 backfills source_priority from the source string.
    /// We construct a v1 database by hand, then call `migrate` and verify
    /// the column exists and is populated correctly.
    #[test]
    fn migrate_v1_to_v2_backfills_source_priority() {
        let conn = Connection::open_in_memory().unwrap();
        // Build a v1 schema and seed it.
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version VALUES (1);
             CREATE TABLE aps (
                 bssid TEXT PRIMARY KEY, ssid TEXT, lat REAL NOT NULL, lon REAL NOT NULL,
                 encryption TEXT, channel INTEGER, frequency INTEGER, city TEXT, country TEXT,
                 source TEXT NOT NULL, first_seen TEXT NOT NULL, last_seen TEXT NOT NULL,
                 fetched_at TEXT NOT NULL
             );
             INSERT INTO aps VALUES ('A','x',1,1,NULL,NULL,NULL,NULL,NULL,'apple','t','t','t');
             INSERT INTO aps VALUES ('B','x',1,1,NULL,NULL,NULL,NULL,NULL,'wigle','t','t','t');
             INSERT INTO aps VALUES ('C','x',1,1,NULL,NULL,NULL,NULL,NULL,'beacondb','t','t','t');
             INSERT INTO aps VALUES ('D','x',1,1,NULL,NULL,NULL,NULL,NULL,'manual','t','t','t');
             INSERT INTO aps VALUES ('E','x',1,1,NULL,NULL,NULL,NULL,NULL,'mystery','t','t','t');
             CREATE TABLE not_found (bssid TEXT PRIMARY KEY, first_seen TEXT, last_seen TEXT, checked_at TEXT);
             CREATE TABLE pending (bssid TEXT PRIMARY KEY, ssid TEXT, channel INTEGER, frequency INTEGER, signal_dbm INTEGER, first_seen TEXT, last_seen TEXT, attempts INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        let db = Database { conn };
        db.migrate().unwrap();
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);

        let priorities: Vec<(String, i32)> = {
            let mut stmt = db
                .conn
                .prepare("SELECT bssid, source_priority FROM aps ORDER BY bssid")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert_eq!(
            priorities,
            vec![
                ("A".to_string(), 40), // apple
                ("B".to_string(), 30), // wigle
                ("C".to_string(), 20), // beacondb
                ("D".to_string(), 10), // manual
                ("E".to_string(), 0),  // unknown
            ]
        );
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

    /// `is_pending` must reflect the same state as `get_pending` but cheaply.
    /// We exercise the three transitions: insert, delete, and a never-seen
    /// BSSID. The indexed PRIMARY KEY probe means this stays O(log n) even
    /// with a large pending queue.
    #[test]
    fn is_pending_reflects_queue_membership() {
        let db = Database::open_memory().unwrap();
        let bssid = "AA:BB:CC:DD:EE:FF";
        let other = "11:22:33:44:55:66";

        // Empty queue: neither BSSID is pending.
        assert!(!db.is_pending(bssid).unwrap());
        assert!(!db.is_pending(other).unwrap());

        // After insert: only the inserted one is pending.
        db.insert_pending(bssid, Some("Test"), Some(6), Some(2437), Some(-65))
            .unwrap();
        assert!(db.is_pending(bssid).unwrap());
        assert!(!db.is_pending(other).unwrap());

        // After delete: pending again returns false.
        db.delete_pending(bssid).unwrap();
        assert!(!db.is_pending(bssid).unwrap());
    }

    #[test]
    fn test_rate_limiting() {
        let mut db = Database::open_memory().unwrap();
        assert_eq!(db.api_calls_today().unwrap(), 0);

        // First reservation under generous limit succeeds.
        assert!(db.try_reserve_api_call(100).unwrap());
        assert_eq!(db.api_calls_today().unwrap(), 1);

        // Limit-of-1 path: already at 1, so further reservation denied.
        assert!(!db.try_reserve_api_call(1).unwrap());
        // Counter unchanged after a denied reservation.
        assert_eq!(db.api_calls_today().unwrap(), 1);

        // Limit-of-2 path: still room for one more.
        assert!(db.try_reserve_api_call(2).unwrap());
        assert_eq!(db.api_calls_today().unwrap(), 2);

        // Refund returns the slot.
        db.refund_api_call().unwrap();
        assert_eq!(db.api_calls_today().unwrap(), 1);

        // Refund clamps at 0.
        db.refund_api_call().unwrap();
        db.refund_api_call().unwrap();
        assert_eq!(db.api_calls_today().unwrap(), 0);
    }

    /// Concurrent reservations must not exceed the daily limit even when
    /// many threads race on the same Database. Database is !Send-shareable
    /// across threads (rusqlite Connection), so we wrap it in a Mutex,
    /// matching the daemon's runtime invariant (DaemonState::db is a Mutex).
    /// The interesting property is: if two threads both held the lock long
    /// enough to read the counter independently, they could both decide to
    /// proceed. With try_reserve_api_call doing its read+increment under a
    /// single SQL transaction (and held under the outer Mutex), that cannot
    /// happen.
    #[test]
    fn test_rate_limiting_concurrent() {
        use std::sync::{Arc, Mutex};
        use std::thread;

        let db = Arc::new(Mutex::new(Database::open_memory().unwrap()));
        let limit: u32 = 10;
        let threads = 64;

        let reserved = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let mut handles = Vec::new();
        for _ in 0..threads {
            let db = Arc::clone(&db);
            let reserved = Arc::clone(&reserved);
            handles.push(thread::spawn(move || {
                let ok = {
                    let mut g = db.lock().unwrap();
                    g.try_reserve_api_call(limit).unwrap()
                };
                if ok {
                    reserved.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let total = reserved.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(total, limit, "total reservations must equal daily_limit");
        assert_eq!(db.lock().unwrap().api_calls_today().unwrap(), limit);
    }

    #[test]
    fn last_fix_round_trip() {
        let db = Database::open_memory().unwrap();
        // Empty initially.
        assert!(db.get_last_fix().unwrap().is_none());

        let row = LastFixRow {
            lat: 55.6761,
            lon: 12.5683,
            accuracy_m: 42.0,
            address: Some("Copenhagen".to_string()),
            at_rfc3339: "2026-05-09T12:34:56+00:00".to_string(),
            sources: 5,
        };
        db.set_last_fix(&row).unwrap();
        let got = db.get_last_fix().unwrap().unwrap();
        assert!((got.lat - row.lat).abs() < 1e-9);
        assert!((got.lon - row.lon).abs() < 1e-9);
        assert!((got.accuracy_m - row.accuracy_m).abs() < 1e-9);
        assert_eq!(got.address, row.address);
        assert_eq!(got.at_rfc3339, row.at_rfc3339);
        assert_eq!(got.sources, row.sources);

        // Overwrite must work and stay single-row.
        let updated = LastFixRow {
            lat: 48.8566,
            lon: 2.3522,
            accuracy_m: 99.0,
            address: None,
            at_rfc3339: "2026-05-10T00:00:00+00:00".to_string(),
            sources: 7,
        };
        db.set_last_fix(&updated).unwrap();
        let got = db.get_last_fix().unwrap().unwrap();
        assert!((got.lat - 48.8566).abs() < 1e-9);
        assert_eq!(got.address, None);
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM last_fix", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "last_fix must remain single-row after overwrite");
    }

    #[test]
    fn last_fix_table_rejects_second_row() {
        // The CHECK (id = 1) constraint must reject any insert with id != 1.
        let db = Database::open_memory().unwrap();
        let row = LastFixRow {
            lat: 0.0,
            lon: 0.0,
            accuracy_m: 1.0,
            address: None,
            at_rfc3339: "2026-05-09T00:00:00+00:00".to_string(),
            sources: 0,
        };
        db.set_last_fix(&row).unwrap();
        let err = db.conn.execute(
            "INSERT INTO last_fix (id, lat, lon, accuracy_m, address, at_rfc3339, sources)
             VALUES (2, 0, 0, 0, NULL, '2026-05-09T00:00:00+00:00', 0)",
            [],
        );
        assert!(
            err.is_err(),
            "INSERT with id=2 must violate CHECK constraint, got {err:?}"
        );
    }

    #[test]
    fn last_fix_survives_db_reopen() {
        // Round-trip through an on-disk DB so we exercise the file-backed path.
        // Avoid adding a tempfile dep — manual cleanup is fine for one test.
        let path = std::env::temp_dir().join(format!(
            "whereamid_last_fix_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let row = LastFixRow {
            lat: 51.5074,
            lon: -0.1278,
            accuracy_m: 25.0,
            address: Some("London".to_string()),
            at_rfc3339: "2026-05-09T08:00:00+00:00".to_string(),
            sources: 3,
        };
        let result = (|| -> Result<()> {
            {
                let db = Database::open(&path)?;
                db.set_last_fix(&row)?;
            }
            // Reopen: rehydration source-of-truth is on disk.
            let db = Database::open(&path)?;
            let got = db
                .get_last_fix()?
                .expect("rehydrated row must be present after reopen");
            assert!((got.lat - row.lat).abs() < 1e-9);
            assert!((got.lon - row.lon).abs() < 1e-9);
            assert_eq!(got.address, row.address);
            Ok(())
        })();
        // Cleanup the .db file plus WAL/SHM siblings even on failure.
        for ext in ["", "-wal", "-shm"] {
            let mut p = path.clone();
            if !ext.is_empty() {
                p.set_file_name(format!(
                    "{}{}",
                    p.file_name().unwrap().to_string_lossy(),
                    ext
                ));
            }
            let _ = std::fs::remove_file(&p);
        }
        result.unwrap();
    }
}
