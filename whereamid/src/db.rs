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
    /// Provenance of the position. Internally a typed enum; the SQLite
    /// boundary stringifies via `Source::as_str` and parses back via
    /// `Source::from_db_str`. No string literals appear in production
    /// callers.
    pub source: Source,
}

// task-0055: Source enum extracted to db/source.rs. Public API
// re-exported here so existing call sites (`crate::db::Source`) keep
// working unchanged.
mod source;
pub use source::Source;

// task-0060: PendingAp struct removed. `get_pending` now returns the
// minimal `(bssid, attempts)` tuple — the four metadata fields the
// struct used to carry (ssid/channel/frequency/signal_dbm) had no
// production consumer and only existed to suppress dead_code lints.
// The SQL `pending` table still records them for debug queries; widen
// the projection (and reintroduce a struct) the moment a real consumer
// appears.

pub struct Database {
    conn: Connection,
}

const SCHEMA_VERSION: i32 = 5;

/// Parse the `metadata.api_calls_today` raw value.
///
/// `None` (key absent) is a legitimate "no calls recorded yet today" and
/// maps to `Ok(0)`. A present-but-non-numeric value is corruption (disk
/// gremlins, partial write) and returns `Err` so callers fail **closed**
/// (task-0097): `try_reserve_api_call` denies the slot rather than silently
/// resetting the counter to 0 and re-charging the full daily WiGLE quota on
/// top of whatever was already spent — the WiGLE budget is irreversible.
/// The `warn` (task-0077) keeps the corruption visible in logs; it now
/// describes the deny action instead of the old "treating as 0" heal.
fn parse_api_calls_value(raw: Option<String>) -> Result<u32> {
    match raw.as_deref() {
        None => Ok(0),
        Some(s) => s.parse::<u32>().map_err(|_| {
            tracing::warn!(
                "metadata.api_calls_today is not a valid u32 ({:?}); \
                 denying API reservation to avoid double-spending the daily quota",
                s
            );
            anyhow::anyhow!("corrupt metadata.api_calls_today value: {s:?}")
        }),
    }
}

/// A historical fix row from the `fixes` table (task-0031). Each row records
/// one successful locate response so the daemon can answer "where was I N
/// days ago" queries by grouping rows into stay-point segments.
///
/// `id` and `n_sources` are read from the DB but the segmentation logic
/// (`history::segment_fixes`) only needs the position+timestamp+accuracy.
/// They're carried for completeness so external consumers (e.g. analysis
/// tools that consume the typed struct) get the full row.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FixRow {
    pub id: i64,
    pub at_rfc3339: String,
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: f64,
    /// Number of cached APs used to compute this fix.
    pub n_sources: i64,
}

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

        // task-0191 AC#3: heal a DB whose stored version is newer than its
        // actual table shape. This is exactly the corruption the old
        // migrate_v2_to_v3 SCHEMA_VERSION-stamping bug could produce: a crash
        // between the v2->v3 commit and the v3->v4 commit left version=5 on
        // disk with neither last_fix (v4) nor fixes (v5) present, after which
        // every later open read 5, skipped both migrations, and stranded the
        // daemon permanently with no self-heal.
        //
        // Repair = reset the stored version DOWN to the highest version whose
        // shape is actually satisfied, then let the normal chain below re-run
        // and rebuild the missing tables. This is safe ONLY because every
        // migration is idempotency-guarded (v1->v2 has_priority probe, v2->v3
        // already_constrained probe, v3->v4/v4->v5 CREATE TABLE IF NOT EXISTS),
        // so re-running heals rather than harms.
        //
        // Guard rail: only repair when version <= SCHEMA_VERSION. A version
        // STRICTLY greater than SCHEMA_VERSION is a genuinely-newer schema
        // written by a future binary; its tables are simply unknown to us and
        // must NOT be "repaired" down (that would corrupt a forward-compatible
        // DB). The forward-compat warning at the end of migrate() handles that
        // case instead. A missing shape can only be observed at v4+ (last_fix)
        // and v5 (fixes); v1..v3 touch only tables create_schema_v1 always
        // creates, so there is no sub-v4 shape to lose.
        if version <= SCHEMA_VERSION {
            let satisfied = self.highest_satisfied_shape_version(version);
            if satisfied < version {
                tracing::warn!(
                    "database stamped v{version} but its table shape only satisfies \
                     v{satisfied} (missing last_fix/fixes); resetting the stored \
                     version down and re-running migrations to heal (task-0191)"
                );
                // A stored version >= 4 implies the schema_version table
                // already has the v3 single-row (id = 1) shape — reaching v4
                // required migrate_v3_to_v4, which addresses the row by id.
                // (This block only fires when satisfied < version, i.e.
                // version >= 4.) A schema_version table manually corrupted to
                // a non-id shape while stamped >= 4 is out of contract; the
                // UPDATE below would then fail loudly, not silently mis-heal.
                self.conn
                    .execute(
                        "UPDATE schema_version SET version = ?1 WHERE id = 1",
                        params![satisfied],
                    )
                    .context("failed to reset schema_version during shape repair")?;
                version = satisfied;
            }
        }

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

        if version < 5 {
            info!("migrating database v{version} -> v5 (fixes timeseries table)");
            self.migrate_v4_to_v5()?;
            version = 5;
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

    /// Run a migration body inside an `unchecked_transaction`. The body MUST
    /// also stamp schema_version (because the stamp shape differs across
    /// versions: pre-v3 is a multi-row table, v3+ has CHECK(id=1)). On any
    /// error the transaction is rolled back and the DB stays at the prior
    /// version. Task-0047.
    fn run_migration<F>(&self, body: F) -> Result<()>
    where
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<()>,
    {
        let tx = self
            .conn
            .unchecked_transaction()
            .context("failed to start migration transaction")?;
        body(&tx)?;
        tx.commit().context("failed to commit migration")?;
        Ok(())
    }

    /// v1 -> v2: add `aps.source_priority` and backfill from the existing
    /// `source` column. Idempotent: safe to run multiple times because the
    /// schema_version row is the gating condition; we never reach this from
    /// a v2 database. Wrapped in a transaction (task-0047) so a partial
    /// migration cannot leave the DB with the column added but no version
    /// bump (or vice versa).
    fn migrate_v1_to_v2(&self) -> Result<()> {
        self.run_migration(|tx| {
            // Defensive: if the column already exists (e.g. partial earlier run),
            // skip the ALTER. We detect that via PRAGMA table_info.
            let has_priority: bool = {
                let mut stmt = tx.prepare("PRAGMA table_info(aps)")?;
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
                tx.execute(
                    "ALTER TABLE aps ADD COLUMN source_priority INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .context("failed to add aps.source_priority column")?;
            }

            // Backfill priorities from existing source strings. Hardcode the
            // numbers here rather than calling Source::priority(): future code
            // changes to the enum must not silently rewrite historical data.
            // (If you change the priority ladder, write a new migration.)
            tx.execute_batch(
                "UPDATE aps SET source_priority = 40 WHERE source = 'apple';
                 UPDATE aps SET source_priority = 30 WHERE source = 'wigle';
                 UPDATE aps SET source_priority = 20 WHERE source = 'beacondb';
                 UPDATE aps SET source_priority = 10 WHERE source = 'manual';
                 UPDATE aps SET source_priority = 0  WHERE source NOT IN ('apple','wigle','beacondb','manual');",
            )?;

            // Bump schema_version. Multi-row shape (no id PK yet). Update if
            // a row exists, otherwise insert.
            let updated = tx.execute("UPDATE schema_version SET version = 2", [])?;
            if updated == 0 {
                tx.execute("INSERT INTO schema_version (version) VALUES (2)", [])?;
            }
            Ok(())
        })
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
            // Idempotent, no-downgrade stamp of the literal 3 — this
            // migration's postcondition version, NOT the moving SCHEMA_VERSION
            // constant (task-0191, mirroring migrate_v1_to_v2's literal 2):
            // stamping SCHEMA_VERSION here strands a crash-interrupted DB at a
            // version whose v4/v5 tables do not exist yet. INSERT OR IGNORE
            // creates the row iff absent (the id = 1 PK makes a second insert a
            // no-op); the guarded UPDATE then advances it to 3 only when it is
            // behind. Together they never downgrade a row a later migration
            // already moved to 4/5 (if this path is ever reached out of order),
            // and never error on a pre-existing row.
            self.conn.execute(
                "INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, 3)",
                [],
            )?;
            self.conn.execute(
                "UPDATE schema_version SET version = 3 WHERE id = 1 AND version < 3",
                [],
            )?;
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

        // Stamp the (now single) row to the literal 3 — this migration's
        // postcondition version, NOT SCHEMA_VERSION (task-0191). The row was
        // just seeded from COALESCE(MAX(old version), 0), which could already
        // be >= 3 if a malformed source held a higher value; the `version < 3`
        // guard keeps that higher value rather than downgrading it. Stamping
        // SCHEMA_VERSION here was the original defect: a crash before the
        // v3->v4 commit left version=5 on disk with no last_fix/fixes table.
        tx.execute(
            "UPDATE schema_version SET version = 3 WHERE id = 1 AND version < 3",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// v3 -> v4: add the `last_fix` table with a single-row CHECK so the
    /// daemon can persist its last-known position across restarts. Idempotent
    /// via `CREATE TABLE IF NOT EXISTS` and a fresh schema_version stamp.
    /// Wrapped in a transaction (task-0047).
    fn migrate_v3_to_v4(&self) -> Result<()> {
        self.run_migration(|tx| {
            tx.execute_batch(
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
            tx.execute(
                "UPDATE schema_version SET version = ?1 WHERE id = 1",
                params![4],
            )?;
            Ok(())
        })
    }

    /// v4 -> v5: add the `fixes` table (timeseries of successful locates)
    /// for the location-history feature (task-0031). Idempotent via
    /// CREATE TABLE / INDEX IF NOT EXISTS. Wrapped in a transaction
    /// (task-0047).
    fn migrate_v4_to_v5(&self) -> Result<()> {
        self.run_migration(|tx| {
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS fixes (
                     id          INTEGER PRIMARY KEY AUTOINCREMENT,
                     at_rfc3339  TEXT NOT NULL,
                     lat         REAL NOT NULL,
                     lon         REAL NOT NULL,
                     accuracy_m  REAL NOT NULL,
                     n_sources   INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_fixes_at ON fixes(at_rfc3339);",
            )
            .context("failed to create fixes table")?;
            tx.execute(
                "UPDATE schema_version SET version = ?1 WHERE id = 1",
                params![5],
            )?;
            Ok(())
        })
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

    /// Does a table named `name` exist? Used by the shape-repair in migrate()
    /// (task-0191). A query error (e.g. a locked or corrupt catalog) is
    /// treated as "absent" — the conservative choice, since the migration
    /// chain that follows re-creates the table idempotently anyway.
    fn table_exists(&self, name: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                params![name],
                |_| Ok(()),
            )
            .optional()
            .map(|o| o.is_some())
            .unwrap_or(false)
    }

    /// task-0191 AC#3: the highest schema version whose on-disk *table shape*
    /// is actually present, capped at the `stamped` version (we never claim a
    /// shape newer than the stored stamp). v4 introduced `last_fix` and v5
    /// introduced `fixes`; v5 layers on v4, so a missing `last_fix` caps us at
    /// 3 even if `fixes` somehow exists. Versions below 4 touch only tables
    /// create_schema_v1 always creates, so anything < 4 is shape-satisfied by
    /// definition and returned unchanged (no sqlite_master probe needed).
    fn highest_satisfied_shape_version(&self, stamped: i32) -> i32 {
        if stamped >= 4 && !self.table_exists("last_fix") {
            return 3;
        }
        if stamped >= 5 && !self.table_exists("fixes") {
            return 4;
        }
        stamped
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
                    let source_str: String = row.get(9)?;
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
                        source: Source::from_db_str(&source_str),
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
        let source_str = ap.source.as_str();
        let priority = ap.source.priority();
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
            params![ap.bssid, ap.ssid, ap.lat, ap.lon, ap.encryption, ap.channel, ap.frequency, ap.city, ap.country, source_str, priority, now],
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

    /// Get up to `limit` pending `(bssid, attempts)` rows, ordered by
    /// fewest attempts first.
    ///
    /// task-0060: returns a tuple instead of the former `PendingAp`
    /// struct. Production callers only need the BSSID; `attempts` is
    /// retained for tests that assert the retry counter advances. Widen
    /// the projection if a real consumer needs ssid/channel/frequency/
    /// signal_dbm.
    pub fn get_pending(&self, limit: usize) -> Result<Vec<(String, i32)>> {
        let mut stmt = self.conn.prepare(
            "SELECT bssid, attempts
             FROM pending ORDER BY attempts ASC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?))
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

    /// Set (insert or overwrite) a metadata value by key.
    pub fn set_metadata(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
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
            let raw: Option<String> = tx
                .query_row(
                    "SELECT value FROM metadata WHERE key = 'api_calls_today'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            match parse_api_calls_value(raw) {
                Ok(v) => v,
                // task-0097: corrupt counter -> fail closed. Deny the slot and
                // return WITHOUT writing. Healing the garbage here would reset
                // the counter and re-charge up to `daily_limit` fresh WiGLE
                // calls on top of whatever was already spent. Dropping `tx`
                // uncommitted rolls back cleanly (no write has happened on this
                // branch). The warn already fired inside parse_api_calls_value.
                Err(_) => return Ok(false),
            }
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
        let raw: Option<String> = tx
            .query_row(
                "SELECT value FROM metadata WHERE key = 'api_calls_today'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        // task-0097: a corrupt counter propagates as Err (tx rolls back, no
        // write). Refunding by healing the garbage to "0" would re-introduce
        // the double-spend heal this task closes, so fail closed here too.
        let count: u32 = parse_api_calls_value(raw)?;
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
        // task-0097: corrupt counter -> Err. The only caller (status display)
        // uses `.unwrap_or(0)`, so a garbage row renders as 0 in stats without
        // affecting the enforcement path in try_reserve_api_call.
        parse_api_calls_value(self.get_metadata("api_calls_today")?)
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

    /// Persist the last-known position into the single-row `last_fix` table.
    ///
    /// **Invariant: monotonic by `at_rfc3339` — older writes are silently
    /// dropped.** The UPSERT overwrites the existing row only when the
    /// incoming `at_rfc3339` is `>=` the stored one (see the trailing
    /// `WHERE`). Concurrent locates are already serialized by the daemon's DB
    /// mutex (task-0046); this guard instead defends against the wall clock
    /// moving backwards (NTP step, container time fix on restart) letting a
    /// stale fix clobber a newer one (task-0100).
    ///
    /// The string `>=` is a valid chronological test only because every
    /// writer produces `DateTime::<Utc>::to_rfc3339()`: a fixed-width
    /// `YYYY-MM-DDTHH:MM:SS` prefix and a constant `+00:00` offset, so
    /// lexicographic order matches time order. (AutoSi variable-width
    /// fractional seconds still sort correctly because the `.` that starts a
    /// fraction sorts after the `+` of the offset.) A non-UTC offset, or a
    /// writer switching timestamp formats, would break this.
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
                sources    = excluded.sources
             WHERE excluded.at_rfc3339 >= last_fix.at_rfc3339",
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

    // --- fixes table (timeseries; task-0031) ---

    /// Append one fix row to the timeseries.
    ///
    /// task-0100: unlike [`Database::set_last_fix`], the `fixes` timeseries
    /// has **no monotonicity guard**. Rows are ordered by the auto-increment
    /// `id` (insertion order), not by `at_rfc3339`, so a wall-clock skew can
    /// append a row whose timestamp predates its neighbour. That is
    /// intentional: a timeseries records observations in the order they
    /// happened. `get_fixes_in_range` still filters and sorts by `at_rfc3339`,
    /// so an out-of-order row lands in its correct time bucket. No DB-side
    /// `CHECK` is added — it would require a schema migration and would reject
    /// legitimate skew-affected observations.
    pub fn insert_fix(
        &self,
        at_rfc3339: &str,
        lat: f64,
        lon: f64,
        accuracy_m: f64,
        n_sources: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO fixes (at_rfc3339, lat, lon, accuracy_m, n_sources)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![at_rfc3339, lat, lon, accuracy_m, n_sources],
        )?;
        Ok(())
    }

    /// Read fix rows whose `at_rfc3339` falls within `[start_rfc3339, end_rfc3339]`,
    /// in ascending time order.
    pub fn get_fixes_in_range(
        &self,
        start_rfc3339: &str,
        end_rfc3339: &str,
    ) -> Result<Vec<FixRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, at_rfc3339, lat, lon, accuracy_m, n_sources
             FROM fixes
             WHERE at_rfc3339 >= ?1 AND at_rfc3339 <= ?2
             ORDER BY at_rfc3339 ASC",
        )?;
        let rows = stmt.query_map(params![start_rfc3339, end_rfc3339], |r| {
            Ok(FixRow {
                id: r.get(0)?,
                at_rfc3339: r.get(1)?,
                lat: r.get(2)?,
                lon: r.get(3)?,
                accuracy_m: r.get(4)?,
                n_sources: r.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Delete fixes older than `retention_days`. Returns the number deleted.
    ///
    /// task-0101: `retention_days == 0` means "retention disabled — keep
    /// forever" (matching the `--history-retention-days` help text), so it
    /// early-returns `Ok(0)` and deletes nothing. Without this guard the
    /// cutoff would equal `now` and every sweep would wipe the entire
    /// history. Negative values are rejected at startup by `Args::validate`
    /// (fail-fast in config.rs); the guard is `<= 0` rather than `== 0` so
    /// that this `pub` DELETE fails *safe* (no-op) rather than *catastrophic*
    /// (a negative would make the cutoff a future time and wipe everything)
    /// even if some future caller bypasses `validate`.
    pub fn prune_fixes(&self, retention_days: i64) -> Result<usize> {
        if retention_days <= 0 {
            return Ok(0);
        }
        let cutoff = (Utc::now() - chrono::TimeDelta::days(retention_days)).to_rfc3339();
        let n = self
            .conn
            .execute("DELETE FROM fixes WHERE at_rfc3339 < ?1", params![cutoff])?;
        Ok(n)
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

    /// Helper: is the schema_version table the v3 single-row shape?
    fn schema_version_is_single_row_constrained(db: &Database) -> bool {
        db.conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='schema_version'",
                [],
                |r| r.get::<_, String>(0),
            )
            .map(|s| s.contains("CHECK (id = 1)"))
            .unwrap_or(false)
    }

    /// Build a genuine v2 database in memory: create_schema_v1 then
    /// migrate_v1_to_v2. Private fns, reachable from this in-module test.
    fn fresh_v2_db() -> Database {
        let db = Database {
            conn: Connection::open_in_memory().unwrap(),
        };
        db.create_schema_v1().unwrap();
        db.migrate_v1_to_v2().unwrap();
        assert_eq!(db.get_schema_version(), 2, "fixture must start at v2");
        db
    }

    /// task-0191 AC#1/#2: step through the migration chain one version at a
    /// time, asserting the stored stamp AND the table shape after EACH step.
    /// This is the direct regression guard for the v2->v3
    /// SCHEMA_VERSION-stamping bug: applied in isolation, migrate_v2_to_v3
    /// must leave the stamp at EXACTLY 3 (not 5), with neither last_fix nor
    /// fixes present yet.
    #[test]
    fn migration_chain_stamps_each_version_in_lockstep() {
        let db = fresh_v2_db();
        // v2 shape: multi-row schema_version, no v4/v5 tables.
        assert!(!db.table_exists("last_fix"));
        assert!(!db.table_exists("fixes"));

        // --- v2 -> v3 (in isolation): stamp is exactly 3, shape is v3 ---
        db.migrate_v2_to_v3().unwrap();
        assert_eq!(
            db.get_schema_version(),
            3,
            "migrate_v2_to_v3 must stamp exactly 3, not SCHEMA_VERSION"
        );
        assert!(
            schema_version_is_single_row_constrained(&db),
            "v3 shape: schema_version must be single-row constrained"
        );
        assert!(
            !db.table_exists("last_fix"),
            "last_fix must NOT exist at v3"
        );
        assert!(!db.table_exists("fixes"), "fixes must NOT exist at v3");

        // --- v3 -> v4: stamp 4, last_fix appears, fixes still absent ---
        db.migrate_v3_to_v4().unwrap();
        assert_eq!(db.get_schema_version(), 4);
        assert!(db.table_exists("last_fix"), "last_fix present at v4");
        assert!(!db.table_exists("fixes"), "fixes still absent at v4");

        // --- v4 -> v5: stamp 5, fixes appears ---
        db.migrate_v4_to_v5().unwrap();
        assert_eq!(db.get_schema_version(), 5);
        assert!(db.table_exists("last_fix"));
        assert!(db.table_exists("fixes"), "fixes present at v5");
    }

    /// task-0191 AC#3 (positive): a DB stamped newer than its shape — the
    /// exact strand the old SCHEMA_VERSION bug produced — is detected and
    /// repaired on migrate(), not silently skipped. We manufacture the strand:
    /// a real v3 DB whose stored version is hand-forced to 5 with no v4/v5
    /// tables. migrate() must reset down, re-run the chain, and rebuild the
    /// missing tables.
    #[test]
    fn migrate_repairs_version_stamped_ahead_of_shape() {
        let db = fresh_v2_db();
        db.migrate_v2_to_v3().unwrap();
        assert_eq!(db.get_schema_version(), 3);
        assert!(!db.table_exists("last_fix"));
        assert!(!db.table_exists("fixes"));

        // Forge the strand: stamp 5 while last_fix/fixes still do not exist.
        db.conn
            .execute("UPDATE schema_version SET version = 5 WHERE id = 1", [])
            .unwrap();
        assert_eq!(db.get_schema_version(), 5);

        // Heal: warn + reset to 3 + re-run v3->v4->v5.
        db.migrate().unwrap();
        assert_eq!(db.get_schema_version(), 5, "healed DB ends at v5");
        assert!(db.table_exists("last_fix"), "repair must create last_fix");
        assert!(db.table_exists("fixes"), "repair must create fixes");
    }

    /// task-0191 AC#3 (negative): an intact v5 DB passes the shape check
    /// untouched — no spurious repair. Re-running migrate() is a no-op:
    /// version stays 5 and both tables remain present.
    #[test]
    fn migrate_does_not_repair_intact_v5() {
        let db = Database::open_memory().unwrap();
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);
        assert!(db.table_exists("last_fix"));
        assert!(db.table_exists("fixes"));

        db.migrate().unwrap();
        assert_eq!(db.get_schema_version(), 5, "intact v5 must stay at 5");
        assert!(db.table_exists("last_fix"));
        assert!(db.table_exists("fixes"));
    }

    /// task-0191 AC#3 guard rail: a genuinely-newer schema from a FUTURE
    /// binary (version > SCHEMA_VERSION) must NOT be repaired down, even
    /// though its unknown tables are "missing" from our point of view.
    /// Forward-compatibility: we leave it exactly as found.
    #[test]
    fn migrate_leaves_future_schema_untouched() {
        let db = Database::open_memory().unwrap();
        db.conn
            .execute("UPDATE schema_version SET version = 99 WHERE id = 1", [])
            .unwrap();
        db.migrate().unwrap();
        assert_eq!(
            db.get_schema_version(),
            99,
            "a version > SCHEMA_VERSION must be left untouched (no down-repair)"
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
            source: Source::Wigle,
        };
        db.upsert_ap(&ap).unwrap();
        let got = db.get_ap("AA:BB:CC:DD:EE:FF").unwrap().unwrap();
        assert_eq!(got.ssid, Some("TestWiFi".to_string()));
        assert!((got.lat - 55.6684).abs() < 1e-6);
        assert_eq!(got.source, Source::Wigle);
    }

    /// Build an ApInfo with the given source and lat (used to tell rows
    /// apart in priority assertions).
    fn ap_with(source: Source, lat: f64) -> ApInfo {
        ApInfo {
            bssid: "AA:BB:CC:DD:EE:FF".to_string(),
            ssid: Some(format!("ssid-{}", source.as_str())),
            lat,
            lon: 12.5541,
            encryption: None,
            channel: Some(6),
            frequency: Some(2437),
            city: None,
            country: None,
            source,
        }
    }

    // source_priority_ladder test moved to db/source.rs alongside the enum.

    /// upsert_ap honours the priority ladder. Exercise every cross-source
    /// transition: equal-priority overwrites, higher-priority overwrites,
    /// lower-priority does NOT overwrite the data fields, but `last_seen`
    /// always advances (an observation is always recorded).
    #[test]
    fn upsert_ap_respects_source_priority() {
        let db = Database::open_memory().unwrap();
        let bssid = "AA:BB:CC:DD:EE:FF";

        // 1. Wigle writes first.
        db.upsert_ap(&ap_with(Source::Wigle, 1.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, Source::Wigle);
        assert!((got.lat - 1.0).abs() < 1e-9);

        // 2. Lower-priority manual write must NOT overwrite.
        db.upsert_ap(&ap_with(Source::Manual, 2.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, Source::Wigle, "manual must not overwrite wigle");
        assert!((got.lat - 1.0).abs() < 1e-9, "lat must not change");

        // 3. Lower-priority beacondb write must NOT overwrite wigle either.
        db.upsert_ap(&ap_with(Source::BeaconDb, 3.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, Source::Wigle);

        // 4. Equal-priority same-source rewrite SHOULD update the position
        //    (we trust the more recent observation from the same source).
        db.upsert_ap(&ap_with(Source::Wigle, 4.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, Source::Wigle);
        assert!(
            (got.lat - 4.0).abs() < 1e-9,
            "same-source rewrite should update lat, got {}",
            got.lat
        );

        // 5. Higher-priority apple write SHOULD overwrite.
        db.upsert_ap(&ap_with(Source::Apple, 5.0)).unwrap();
        let got = db.get_ap(bssid).unwrap().unwrap();
        assert_eq!(got.source, Source::Apple);
        assert!((got.lat - 5.0).abs() < 1e-9);

        // 6. Subsequent lower-priority writes (wigle, beacondb, manual,
        //    unknown) must NOT overwrite apple.
        for (src, lat) in [
            (Source::Wigle, 6.0),
            (Source::BeaconDb, 7.0),
            (Source::Manual, 8.0),
            (Source::Unknown, 9.0),
        ] {
            db.upsert_ap(&ap_with(src, lat)).unwrap();
            let got = db.get_ap(bssid).unwrap().unwrap();
            assert_eq!(
                got.source,
                Source::Apple,
                "{:?} must not overwrite apple",
                src
            );
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
        assert_eq!(pending[0].0, "AA:BB:CC:DD:EE:FF");
        assert_eq!(pending[0].1, 0);

        db.increment_pending_attempts("AA:BB:CC:DD:EE:FF").unwrap();
        let pending = db.get_pending(10).unwrap();
        assert_eq!(pending[0].1, 1);

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

    /// task-0097: a corrupt `api_calls_today` value must fail **closed** —
    /// `try_reserve_api_call` denies the slot and leaves the garbage row
    /// untouched, rather than healing it to 0 and re-charging the full daily
    /// WiGLE quota on top of whatever was already spent.
    #[test]
    fn try_reserve_fails_closed_on_corrupt_counter() {
        let mut db = Database::open_memory().unwrap();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        // Seed a corrupt counter for *today* so try_reserve reads it instead
        // of resetting on a date rollover.
        db.conn
            .execute(
                "INSERT INTO metadata (key, value) VALUES ('api_calls_date', ?1)",
                rusqlite::params![today],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO metadata (key, value) VALUES ('api_calls_today', 'garbage')",
                [],
            )
            .unwrap();

        // Fail closed: deny the reservation despite a generous limit.
        let reserved = db.try_reserve_api_call(100).unwrap();
        assert!(!reserved, "corrupt counter must deny the reservation");

        // The corrupt row must NOT be healed/overwritten by the denied call.
        let raw: String = db
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'api_calls_today'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(raw, "garbage", "corrupt counter row must be left untouched");
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

    /// task-0100: set_last_fix is monotonic by at_rfc3339. A write carrying an
    /// OLDER timestamp than the stored row (e.g. after the clock steps
    /// backwards) must be silently dropped, leaving the newer fix intact. A
    /// subsequent newer write must still apply (the guard is `>=`, not `==`).
    #[test]
    fn set_last_fix_drops_older_timestamp() {
        let db = Database::open_memory().unwrap();
        let t2 = LastFixRow {
            lat: 55.0,
            lon: 12.0,
            accuracy_m: 10.0,
            address: Some("newer".to_string()),
            at_rfc3339: "2026-05-10T00:00:00+00:00".to_string(),
            sources: 5,
        };
        db.set_last_fix(&t2).unwrap();

        // Older write (T1 < T2) must NOT overwrite the newer stored fix.
        let t1 = LastFixRow {
            lat: 48.0,
            lon: 2.0,
            accuracy_m: 99.0,
            address: Some("older".to_string()),
            at_rfc3339: "2026-05-09T00:00:00+00:00".to_string(),
            sources: 1,
        };
        db.set_last_fix(&t1).unwrap();

        let got = db.get_last_fix().unwrap().unwrap();
        assert_eq!(
            got.at_rfc3339, "2026-05-10T00:00:00+00:00",
            "older write must be dropped"
        );
        assert!(
            (got.lat - 55.0).abs() < 1e-9,
            "position must remain the newer fix, got lat {}",
            got.lat
        );
        assert_eq!(got.address, Some("newer".to_string()));

        // Sanity: an equal-or-newer write still applies.
        let t3 = LastFixRow {
            lat: 10.0,
            lon: 20.0,
            accuracy_m: 5.0,
            address: Some("newest".to_string()),
            at_rfc3339: "2026-05-11T00:00:00+00:00".to_string(),
            sources: 9,
        };
        db.set_last_fix(&t3).unwrap();
        let got = db.get_last_fix().unwrap().unwrap();
        assert_eq!(got.at_rfc3339, "2026-05-11T00:00:00+00:00");
        assert!((got.lat - 10.0).abs() < 1e-9);

        // Equal-timestamp write MUST apply: the address-backfill path
        // (server.rs) re-persists the same fix with only `address` changed
        // and an unchanged `at_rfc3339`. This pins the `>=` (not `>`) guard —
        // a regression to `>` would silently stop address backfill.
        let t3_backfill = LastFixRow {
            address: Some("backfilled".to_string()),
            ..t3.clone()
        };
        db.set_last_fix(&t3_backfill).unwrap();
        let got = db.get_last_fix().unwrap().unwrap();
        assert_eq!(
            got.address,
            Some("backfilled".to_string()),
            "equal-timestamp write must apply (address backfill depends on >=)"
        );
    }

    /// task-0047: a migration body that returns Err must roll back the
    /// transaction, leaving the DB at the prior schema version with no
    /// partial DDL applied.
    #[test]
    fn migration_rollback_on_error() {
        let db = Database::open_memory().unwrap();
        // Baseline: fully migrated DB at SCHEMA_VERSION.
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);

        // Snapshot the table list pre-attempt.
        let tables_before: Vec<String> = {
            let mut stmt = db
                .conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
            rows.map(|r| r.unwrap()).collect()
        };

        // Run a migration body that creates a table then errors. The CREATE
        // must NOT survive after the helper returns.
        let result: Result<()> = db.run_migration(|tx| {
            tx.execute_batch("CREATE TABLE rollback_target (id INTEGER PRIMARY KEY);")?;
            anyhow::bail!("simulated mid-migration failure");
        });
        assert!(result.is_err(), "expected helper to surface body error");

        let tables_after: Vec<String> = {
            let mut stmt = db
                .conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
            rows.map(|r| r.unwrap()).collect()
        };
        assert_eq!(
            tables_before, tables_after,
            "rollback_target should not exist after failed migration"
        );
        // schema_version still at SCHEMA_VERSION (never bumped).
        assert_eq!(db.get_schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn fixes_round_trip_and_range_query() {
        let db = Database::open_memory().unwrap();
        // Three fixes spread across a day.
        db.insert_fix("2026-05-09T08:00:00+00:00", 55.0, 12.0, 30.0, 5)
            .unwrap();
        db.insert_fix("2026-05-09T12:00:00+00:00", 55.001, 12.001, 25.0, 6)
            .unwrap();
        db.insert_fix("2026-05-09T16:00:00+00:00", 55.0, 12.0, 35.0, 4)
            .unwrap();

        // Query whole day: should return all 3 in ascending order.
        let all = db
            .get_fixes_in_range("2026-05-09T00:00:00+00:00", "2026-05-09T23:59:59+00:00")
            .unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].at_rfc3339, "2026-05-09T08:00:00+00:00");
        assert_eq!(all[2].at_rfc3339, "2026-05-09T16:00:00+00:00");

        // Narrow query to the middle hour: only the 12:00 fix.
        let mid = db
            .get_fixes_in_range("2026-05-09T11:00:00+00:00", "2026-05-09T13:00:00+00:00")
            .unwrap();
        assert_eq!(mid.len(), 1);
        assert!((mid[0].lat - 55.001).abs() < 1e-9);
    }

    #[test]
    fn fixes_prune_drops_old_rows() {
        let db = Database::open_memory().unwrap();
        // One row 100 days old, one row "now".
        let old_ts = (chrono::Utc::now() - chrono::TimeDelta::days(100)).to_rfc3339();
        let new_ts = chrono::Utc::now().to_rfc3339();
        db.insert_fix(&old_ts, 0.0, 0.0, 1.0, 1).unwrap();
        db.insert_fix(&new_ts, 1.0, 1.0, 1.0, 1).unwrap();

        let pruned = db.prune_fixes(30).unwrap();
        assert_eq!(pruned, 1, "expected 1 row pruned");

        let remaining = db
            .get_fixes_in_range("2000-01-01T00:00:00+00:00", "2099-01-01T00:00:00+00:00")
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert!((remaining[0].lat - 1.0).abs() < 1e-9);
    }

    /// task-0101: retention_days == 0 disables pruning (keep forever). It must
    /// be a no-op even when every row is far older than "now" — the previous
    /// behaviour computed cutoff == now and deleted them all on every sweep.
    #[test]
    fn prune_fixes_zero_retention_is_noop() {
        let db = Database::open_memory().unwrap();
        let old_ts = (chrono::Utc::now() - chrono::TimeDelta::days(365)).to_rfc3339();
        for _ in 0..10 {
            db.insert_fix(&old_ts, 1.0, 2.0, 3.0, 1).unwrap();
        }

        let pruned = db.prune_fixes(0).unwrap();
        assert_eq!(pruned, 0, "retention 0 must delete nothing");

        let remaining = db
            .get_fixes_in_range("2000-01-01T00:00:00+00:00", "2099-01-01T00:00:00+00:00")
            .unwrap();
        assert_eq!(
            remaining.len(),
            10,
            "no fixes should be deleted with retention 0"
        );

        // Defense-in-depth: a negative retention must also be a no-op, not a
        // full wipe (cutoff would otherwise be in the future). validate()
        // rejects negatives upstream, but this pub DELETE must fail safe.
        let pruned_neg = db.prune_fixes(-5).unwrap();
        assert_eq!(pruned_neg, 0, "negative retention must delete nothing");
        let remaining = db
            .get_fixes_in_range("2000-01-01T00:00:00+00:00", "2099-01-01T00:00:00+00:00")
            .unwrap();
        assert_eq!(
            remaining.len(),
            10,
            "negative retention must not wipe history"
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
