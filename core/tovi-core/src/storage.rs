//! Local storage (TASKS §9): trusted devices, transfer history, settings.
//!
//! One SQLite database per device (`tovi.db` in the data directory). Schema
//! changes are numbered [`MIGRATIONS`], tracked with `PRAGMA user_version`;
//! a database from a newer TOVI is refused rather than downgraded.
//!
//! Deliberately *not* stored here:
//! - per-chunk receive progress: it lives in the `.tovi.state` file beside
//!   each partial file, so the two can never disagree
//! - pairing sessions: their secrets are single-use and must never touch disk

use crate::identity::DeviceId;
use crate::transfer::TransferId;
use crate::trust::{TrustStore, TrustedDevice};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Schema migrations, applied in order; index + 1 is the schema version.
/// Never edit a released migration: add a new one.
const MIGRATIONS: &[&str] = &[
    // 1: devices, transfer history, settings
    r#"
    CREATE TABLE devices (
        id          BLOB PRIMARY KEY CHECK (length(id) = 32),
        name        TEXT NOT NULL,
        platform    TEXT NOT NULL,
        paired_at   INTEGER NOT NULL,
        last_seen   INTEGER,
        -- Last known "ip:port" endpoints, comma-separated, best first
        addresses   TEXT NOT NULL DEFAULT ''
    ) STRICT;

    -- No foreign key to devices: history outlives forgetting a device
    CREATE TABLE transfers (
        id              BLOB PRIMARY KEY CHECK (length(id) = 16),
        device_id       BLOB NOT NULL CHECK (length(device_id) = 32),
        direction       TEXT NOT NULL CHECK (direction IN ('sent', 'received')),
        file_name       TEXT NOT NULL,
        file_size       INTEGER NOT NULL CHECK (file_size >= 0),
        status          TEXT NOT NULL CHECK (status IN ('in_progress', 'completed', 'failed')),
        -- Source file (sent) or saved file (received)
        path            TEXT,
        -- Sender only: source modified time, ns since 1970, to detect changes on resume
        source_modified INTEGER,
        file_hash       BLOB CHECK (file_hash IS NULL OR length(file_hash) = 32),
        error           TEXT,
        started_at      INTEGER NOT NULL,
        finished_at     INTEGER
    ) STRICT;
    CREATE INDEX transfers_by_start ON transfers (started_at DESC);
    CREATE INDEX transfers_unfinished_sends ON transfers (device_id, path)
        WHERE direction = 'sent' AND status = 'in_progress';

    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) STRICT;
    "#,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Sent,
    Received,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStatus {
    InProgress,
    Completed,
    Failed,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Direction::Sent => "sent",
            Direction::Received => "received",
        }
    }
}

/// A transfer as it starts
#[derive(Debug, Clone)]
pub struct NewTransfer {
    pub id: TransferId,
    pub device_id: DeviceId,
    pub direction: Direction,
    pub file_name: String,
    pub file_size: u64,
    pub path: Option<PathBuf>,
    pub source_modified: Option<SystemTime>,
}

/// One row of transfer history
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRecord {
    pub id: TransferId,
    pub device_id: DeviceId,
    pub direction: Direction,
    pub file_name: String,
    pub file_size: u64,
    pub status: TransferStatus,
    pub path: Option<PathBuf>,
    pub file_hash: Option<blake3::Hash>,
    pub error: Option<String>,
    /// Unix seconds
    pub started_at: u64,
    pub finished_at: Option<u64>,
}

/// A paired device with what the store knows about reaching it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDetails {
    pub device: TrustedDevice,
    /// Unix seconds
    pub paired_at: u64,
    pub last_seen: Option<u64>,
    /// Best first
    pub addresses: Vec<SocketAddr>,
}

/// Addresses kept per device
const MAX_DEVICE_ADDRESSES: usize = 8;

/// A received file the store knows was saved
#[derive(Debug, Clone)]
pub struct CompletedReceive {
    pub path: PathBuf,
    pub file_size: u64,
    pub file_hash: blake3::Hash,
}

pub struct Store {
    conn: Mutex<Connection>,
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn nanos_since_epoch(time: SystemTime) -> Option<i64> {
    let nanos = time.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    i64::try_from(nanos).ok()
}

fn size_to_sql(size: u64) -> Result<i64> {
    i64::try_from(size).context("file too large to record")
}

impl Store {
    /// Open (creating if needed) the database at `path` and bring its schema up to date
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        // WAL lets a reader and a writer (e.g. two TOVI processes) work at once
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
        Self::init(conn)
    }

    /// A throwaway database, for tests
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let conn = self.conn.lock().unwrap();
        Ok(f(&conn)?)
    }

    pub fn schema_version(&self) -> Result<usize> {
        self.with_conn(|c| c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)))
            .map(|v| v as usize)
    }

    // ------------------------------------------------------------ devices

    /// Remember where a device was last reachable, best address first
    pub fn set_device_addresses(&self, id: &DeviceId, addresses: &[SocketAddr]) -> Result<()> {
        let joined = addresses
            .iter()
            .map(SocketAddr::to_string)
            .collect::<Vec<_>>()
            .join(",");
        self.with_conn(|c| {
            c.execute(
                "UPDATE devices SET addresses = ?1 WHERE id = ?2",
                params![joined, id.as_bytes().as_slice()],
            )
        })?;
        Ok(())
    }

    pub fn device_addresses(&self, id: &DeviceId) -> Result<Vec<SocketAddr>> {
        let joined: Option<String> = self.with_conn(|c| {
            c.query_row(
                "SELECT addresses FROM devices WHERE id = ?1",
                [id.as_bytes().as_slice()],
                |r| r.get(0),
            )
            .optional()
        })?;
        Ok(joined
            .unwrap_or_default()
            .split(',')
            .filter_map(|a| a.parse().ok())
            .collect())
    }

    /// Put `address` first in a device's address list (the device was just
    /// reached there), keeping the others as fallbacks
    pub fn remember_address(&self, id: &DeviceId, address: SocketAddr) -> Result<()> {
        let mut addresses = vec![address];
        addresses.extend(
            self.device_addresses(id)?
                .into_iter()
                .filter(|a| *a != address),
        );
        addresses.truncate(MAX_DEVICE_ADDRESSES);
        self.set_device_addresses(id, &addresses)
    }

    /// Every paired device with when it paired, was last seen, and its addresses
    pub fn device_details(&self) -> Result<Vec<DeviceDetails>> {
        let rows = self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, name, platform, paired_at, last_seen, addresses
                 FROM devices ORDER BY paired_at, rowid",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter()
            .map(|(id, name, platform, paired_at, last_seen, addresses)| {
                Ok(DeviceDetails {
                    device: TrustedDevice {
                        id: DeviceId::try_from(id.as_slice())?,
                        name,
                        platform,
                    },
                    paired_at: u64::try_from(paired_at).unwrap_or(0),
                    last_seen: last_seen.and_then(|t| u64::try_from(t).ok()),
                    addresses: addresses
                        .split(',')
                        .filter_map(|a| a.parse().ok())
                        .collect(),
                })
            })
            .collect()
    }

    /// Note that a trusted device just connected
    pub fn touch_device(&self, id: &DeviceId) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE devices SET last_seen = ?1 WHERE id = ?2",
                params![unix_now(), id.as_bytes().as_slice()],
            )
        })?;
        Ok(())
    }

    /// Find a trusted device by name (case-insensitive) or by the start of its
    /// ID (at least 4 hex characters). Errors if the query matches several.
    pub fn find_device(&self, query: &str) -> Result<Option<TrustedDevice>> {
        let query = query.trim();
        let devices = self.list()?;

        let by_name: Vec<_> = devices
            .iter()
            .filter(|d| d.name.eq_ignore_ascii_case(query))
            .collect();
        match by_name.as_slice() {
            [one] => return Ok(Some((*one).clone())),
            [] => {}
            many => bail!(
                "{} devices are named {query:?}; use the device ID",
                many.len()
            ),
        }

        let prefix = query.to_ascii_lowercase();
        if prefix.len() < 4 || !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(None);
        }
        let by_id: Vec<_> = devices
            .iter()
            .filter(|d| d.id.to_string().starts_with(&prefix))
            .collect();
        match by_id.as_slice() {
            [one] => Ok(Some((*one).clone())),
            [] => Ok(None),
            many => bail!(
                "{} devices have IDs starting {prefix}; type more of it",
                many.len()
            ),
        }
    }

    // ---------------------------------------------------------- transfers

    /// Record a transfer starting. Resuming an already-recorded transfer
    /// marks it in progress again and keeps its original start time.
    pub fn record_transfer_started(&self, t: &NewTransfer) -> Result<()> {
        let size = size_to_sql(t.file_size)?;
        let path = t.path.as_ref().map(|p| p.to_string_lossy().into_owned());
        let modified = t.source_modified.and_then(nanos_since_epoch);
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO transfers
                     (id, device_id, direction, file_name, file_size, status, path,
                      source_modified, started_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'in_progress', ?6, ?7, ?8)
                 ON CONFLICT (id) DO UPDATE SET
                     status = 'in_progress', error = NULL, finished_at = NULL",
                params![
                    t.id.as_slice(),
                    t.device_id.as_bytes().as_slice(),
                    t.direction.as_str(),
                    t.file_name,
                    size,
                    path,
                    modified,
                    unix_now()
                ],
            )
        })?;
        Ok(())
    }

    /// Record a transfer finishing successfully
    pub fn record_transfer_completed(
        &self,
        id: &TransferId,
        path: Option<&Path>,
        file_hash: &blake3::Hash,
    ) -> Result<()> {
        let path = path.map(|p| p.to_string_lossy().into_owned());
        self.with_conn(|c| {
            c.execute(
                "UPDATE transfers
                 SET status = 'completed', path = COALESCE(?2, path), file_hash = ?3,
                     error = NULL, finished_at = ?4
                 WHERE id = ?1",
                params![
                    id.as_slice(),
                    path,
                    file_hash.as_bytes().as_slice(),
                    unix_now()
                ],
            )
        })?;
        Ok(())
    }

    /// Record a transfer failing for good (not a resumable drop)
    pub fn record_transfer_failed(&self, id: &TransferId, error: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE transfers SET status = 'failed', error = ?2, finished_at = ?3 WHERE id = ?1",
                params![id.as_slice(), error, unix_now()],
            )
        })?;
        Ok(())
    }

    /// Most recent transfers first
    pub fn history(&self, limit: usize) -> Result<Vec<TransferRecord>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, device_id, direction, file_name, file_size, status, path,
                        file_hash, error, started_at, finished_at
                 FROM transfers ORDER BY started_at DESC, rowid DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit], |r| Ok(RawTransfer::from_row(r)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter().map(|raw| raw?.into_record()).collect()
    }

    pub fn transfer(&self, id: &TransferId) -> Result<Option<TransferRecord>> {
        let raw = self.with_conn(|c| {
            c.query_row(
                "SELECT id, device_id, direction, file_name, file_size, status, path,
                        file_hash, error, started_at, finished_at
                 FROM transfers WHERE id = ?1",
                [id.as_slice()],
                |r| Ok(RawTransfer::from_row(r)),
            )
            .optional()
        })?;
        raw.map(|raw| raw?.into_record()).transpose()
    }

    /// A file already received and saved for transfer `id` from `sender`.
    /// Lets the receiver confirm a retried transfer instead of saving it
    /// twice, even after a restart.
    pub fn completed_receive(
        &self,
        id: &TransferId,
        sender: &DeviceId,
    ) -> Result<Option<CompletedReceive>> {
        let record = match self.transfer(id)? {
            Some(r) => r,
            None => return Ok(None),
        };
        let matches = record.direction == Direction::Received
            && record.status == TransferStatus::Completed
            && record.device_id == *sender;
        Ok(match (matches, record.path, record.file_hash) {
            (true, Some(path), Some(file_hash)) => Some(CompletedReceive {
                path,
                file_size: record.file_size,
                file_hash,
            }),
            _ => None,
        })
    }

    /// The ID of an unfinished send of this exact file (same path, size and
    /// modified time) to `device`, so a restarted sender can resume it
    pub fn unfinished_send(
        &self,
        device: &DeviceId,
        path: &Path,
        file_size: u64,
        modified: Option<SystemTime>,
    ) -> Result<Option<TransferId>> {
        let Some(modified) = modified.and_then(nanos_since_epoch) else {
            return Ok(None); // can't tell whether the file changed
        };
        let size = size_to_sql(file_size)?;
        let id: Option<Vec<u8>> = self.with_conn(|c| {
            c.query_row(
                "SELECT id FROM transfers
                 WHERE direction = 'sent' AND status = 'in_progress'
                   AND device_id = ?1 AND path = ?2 AND file_size = ?3 AND source_modified = ?4
                 ORDER BY started_at DESC LIMIT 1",
                params![
                    device.as_bytes().as_slice(),
                    path.to_string_lossy(),
                    size,
                    modified
                ],
                |r| r.get(0),
            )
            .optional()
        })?;
        id.map(|id| {
            id.try_into()
                .map_err(|_| anyhow::anyhow!("corrupt transfer ID in database"))
        })
        .transpose()
    }

    // ----------------------------------------------------------- settings

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        self.with_conn(|c| {
            c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()
        })
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
        })?;
        Ok(())
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let version = usize::try_from(version).context("negative schema version")?;
    ensure!(
        version <= MIGRATIONS.len(),
        "database schema v{version} is newer than this TOVI build (v{}); update TOVI",
        MIGRATIONS.len()
    );
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("applying schema migration {}", index + 1))?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

/// A `transfers` row before validation
struct RawTransfer {
    id: Vec<u8>,
    device_id: Vec<u8>,
    direction: String,
    file_name: String,
    file_size: i64,
    status: String,
    path: Option<String>,
    file_hash: Option<Vec<u8>>,
    error: Option<String>,
    started_at: i64,
    finished_at: Option<i64>,
}

impl RawTransfer {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            device_id: r.get(1)?,
            direction: r.get(2)?,
            file_name: r.get(3)?,
            file_size: r.get(4)?,
            status: r.get(5)?,
            path: r.get(6)?,
            file_hash: r.get(7)?,
            error: r.get(8)?,
            started_at: r.get(9)?,
            finished_at: r.get(10)?,
        })
    }

    fn into_record(self) -> Result<TransferRecord> {
        let corrupt = || anyhow::anyhow!("corrupt transfer row in database");
        Ok(TransferRecord {
            id: self.id.try_into().map_err(|_| corrupt())?,
            device_id: DeviceId::try_from(self.device_id.as_slice())?,
            direction: match self.direction.as_str() {
                "sent" => Direction::Sent,
                "received" => Direction::Received,
                _ => return Err(corrupt()),
            },
            file_name: self.file_name,
            file_size: u64::try_from(self.file_size).map_err(|_| corrupt())?,
            status: match self.status.as_str() {
                "in_progress" => TransferStatus::InProgress,
                "completed" => TransferStatus::Completed,
                "failed" => TransferStatus::Failed,
                _ => return Err(corrupt()),
            },
            path: self.path.map(PathBuf::from),
            file_hash: self
                .file_hash
                .map(|h| <[u8; 32]>::try_from(h).map(blake3::Hash::from_bytes))
                .transpose()
                .map_err(|_| corrupt())?,
            error: self.error,
            started_at: u64::try_from(self.started_at).unwrap_or(0),
            finished_at: self.finished_at.and_then(|t| u64::try_from(t).ok()),
        })
    }
}

impl TrustStore for Store {
    fn is_trusted(&self, id: &DeviceId) -> bool {
        let found = self.with_conn(|c| {
            c.query_row(
                "SELECT 1 FROM devices WHERE id = ?1",
                [id.as_bytes().as_slice()],
                |_| Ok(()),
            )
            .optional()
        });
        match found {
            Ok(found) => found.is_some(),
            Err(e) => {
                tracing::error!("trust check failed, refusing device: {e:#}");
                false
            }
        }
    }

    fn add(&self, device: TrustedDevice) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO devices (id, name, platform, paired_at, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT (id) DO UPDATE SET
                     name = excluded.name, platform = excluded.platform,
                     last_seen = excluded.last_seen",
                params![
                    device.id.as_bytes().as_slice(),
                    device.name,
                    device.platform,
                    unix_now()
                ],
            )
        })?;
        Ok(())
    }

    fn remove(&self, id: &DeviceId) -> Result<bool> {
        let removed = self.with_conn(|c| {
            c.execute(
                "DELETE FROM devices WHERE id = ?1",
                [id.as_bytes().as_slice()],
            )
        })?;
        Ok(removed > 0)
    }

    fn list(&self) -> Result<Vec<TrustedDevice>> {
        let rows = self.with_conn(|c| {
            let mut stmt =
                c.prepare("SELECT id, name, platform FROM devices ORDER BY paired_at, rowid")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter()
            .map(|(id, name, platform)| {
                Ok(TrustedDevice {
                    id: DeviceId::try_from(id.as_slice())?,
                    name,
                    platform,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;
    use rand_core::{OsRng, RngCore};

    fn device(name: &str) -> TrustedDevice {
        TrustedDevice {
            id: DeviceIdentity::generate_new().device_id(),
            name: name.into(),
            platform: "android".into(),
        }
    }

    fn transfer_id() -> TransferId {
        let mut id = [0u8; 16];
        OsRng.fill_bytes(&mut id);
        id
    }

    fn temp_db() -> (PathBuf, PathBuf) {
        let mut id = [0u8; 8];
        OsRng.fill_bytes(&mut id);
        let dir = std::env::temp_dir().join(format!("tovi-store-test-{}", u64::from_le_bytes(id)));
        (dir.join("tovi.db"), dir)
    }

    fn new_transfer(device: &DeviceId, direction: Direction) -> NewTransfer {
        NewTransfer {
            id: transfer_id(),
            device_id: *device,
            direction,
            file_name: "photo.jpg".into(),
            file_size: 1234,
            path: Some(PathBuf::from("/photos/photo.jpg")),
            source_modified: Some(UNIX_EPOCH + Duration::from_nanos(1_700_000_000_123_456_789)),
        }
    }

    #[test]
    fn schema_is_created_once_and_survives_reopening() {
        let (path, dir) = temp_db();
        let phone = device("Pixel");
        {
            let store = Store::open(&path).unwrap();
            assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len());
            store.add(phone.clone()).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len());
        assert!(store.is_trusted(&phone.id));
        assert_eq!(store.list().unwrap(), vec![phone]);
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn newer_schema_is_refused() {
        let (path, dir) = temp_db();
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        drop(conn);

        let err = Store::open(&path).err().unwrap();
        assert!(err.to_string().contains("newer"), "{err:#}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn trust_add_update_and_revoke() {
        let store = Store::open_in_memory().unwrap();
        let mut phone = device("Pixel");
        store.add(phone.clone()).unwrap();

        phone.name = "Pixel 9".into();
        store.add(phone.clone()).unwrap();
        assert_eq!(store.list().unwrap(), vec![phone.clone()]);

        assert!(store.remove(&phone.id).unwrap());
        assert!(!store.is_trusted(&phone.id));
        assert!(!store.remove(&phone.id).unwrap());
    }

    #[test]
    fn remembered_address_goes_first_without_duplicates() {
        let store = Store::open_in_memory().unwrap();
        let phone = device("Pixel");
        store.add(phone.clone()).unwrap();
        let a: SocketAddr = "192.168.1.20:48210".parse().unwrap();
        let b: SocketAddr = "192.168.1.31:50000".parse().unwrap();

        store.remember_address(&phone.id, a).unwrap();
        store.remember_address(&phone.id, b).unwrap();
        store.remember_address(&phone.id, a).unwrap();
        assert_eq!(store.device_addresses(&phone.id).unwrap(), vec![a, b]);

        for port in 1..=20 {
            store
                .remember_address(&phone.id, SocketAddr::new(a.ip(), port))
                .unwrap();
        }
        assert_eq!(
            store.device_addresses(&phone.id).unwrap().len(),
            MAX_DEVICE_ADDRESSES
        );
    }

    #[test]
    fn device_details_include_last_seen_and_addresses() {
        let store = Store::open_in_memory().unwrap();
        let phone = device("Pixel");
        store.add(phone.clone()).unwrap();
        let addr: SocketAddr = "10.0.0.5:48210".parse().unwrap();
        store.set_device_addresses(&phone.id, &[addr]).unwrap();
        store.touch_device(&phone.id).unwrap();

        let details = store.device_details().unwrap();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].device, phone);
        assert_eq!(details[0].addresses, vec![addr]);
        assert!(details[0].paired_at > 0);
        assert!(details[0].last_seen.is_some());
    }

    #[test]
    fn addresses_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let desk = device("Desk");
        store.add(desk.clone()).unwrap();
        assert!(store.device_addresses(&desk.id).unwrap().is_empty());

        let addrs: Vec<SocketAddr> = vec![
            "192.168.1.20:48210".parse().unwrap(),
            "10.0.0.5:48210".parse().unwrap(),
        ];
        store.set_device_addresses(&desk.id, &addrs).unwrap();
        assert_eq!(store.device_addresses(&desk.id).unwrap(), addrs);
    }

    #[test]
    fn find_device_by_name_or_id_prefix() {
        let store = Store::open_in_memory().unwrap();
        let laptop = device("Obioma's MacBook");
        let phone = device("Pixel");
        store.add(laptop.clone()).unwrap();
        store.add(phone.clone()).unwrap();

        assert_eq!(
            store.find_device("obioma's macbook").unwrap(),
            Some(laptop.clone())
        );
        let prefix = &phone.id.to_string()[..6];
        assert_eq!(store.find_device(prefix).unwrap(), Some(phone.clone()));
        assert_eq!(store.find_device("abc").unwrap(), None); // too short to be an ID
        assert_eq!(store.find_device("Nobody").unwrap(), None);

        store
            .add(TrustedDevice {
                name: "Pixel".into(),
                ..device("x")
            })
            .unwrap();
        assert!(store.find_device("Pixel").is_err()); // ambiguous name
    }

    #[test]
    fn history_records_a_transfer_lifecycle() {
        let store = Store::open_in_memory().unwrap();
        let phone = device("Pixel").id;
        let first = new_transfer(&phone, Direction::Received);
        let second = new_transfer(&phone, Direction::Sent);
        store.record_transfer_started(&first).unwrap();
        store.record_transfer_started(&second).unwrap();

        let hash = blake3::hash(b"contents");
        store
            .record_transfer_completed(&first.id, Some(Path::new("/inbox/photo.jpg")), &hash)
            .unwrap();
        store
            .record_transfer_failed(&second.id, "declined")
            .unwrap();

        let history = store.history(10).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, second.id); // newest first
        assert_eq!(history[0].status, TransferStatus::Failed);
        assert_eq!(history[0].error.as_deref(), Some("declined"));
        let done = &history[1];
        assert_eq!(done.status, TransferStatus::Completed);
        assert_eq!(done.path.as_deref(), Some(Path::new("/inbox/photo.jpg")));
        assert_eq!(done.file_hash, Some(hash));
        assert!(done.finished_at.is_some());
        assert_eq!(store.history(1).unwrap().len(), 1);
    }

    #[test]
    fn restarting_a_recorded_transfer_reopens_it() {
        let store = Store::open_in_memory().unwrap();
        let t = new_transfer(&device("Pixel").id, Direction::Sent);
        store.record_transfer_started(&t).unwrap();
        store.record_transfer_failed(&t.id, "network").unwrap();
        store.record_transfer_started(&t).unwrap();

        let record = store.transfer(&t.id).unwrap().unwrap();
        assert_eq!(record.status, TransferStatus::InProgress);
        assert_eq!(record.error, None);
        assert_eq!(store.history(10).unwrap().len(), 1);
    }

    #[test]
    fn completed_receive_matches_only_the_same_sender() {
        let store = Store::open_in_memory().unwrap();
        let sender = device("Pixel").id;
        let t = new_transfer(&sender, Direction::Received);
        store.record_transfer_started(&t).unwrap();
        assert!(store.completed_receive(&t.id, &sender).unwrap().is_none()); // not finished yet

        let hash = blake3::hash(b"contents");
        store
            .record_transfer_completed(&t.id, Some(Path::new("/inbox/photo.jpg")), &hash)
            .unwrap();
        let done = store.completed_receive(&t.id, &sender).unwrap().unwrap();
        assert_eq!(done.file_hash, hash);
        assert_eq!(done.file_size, 1234);
        assert!(store
            .completed_receive(&t.id, &device("Other").id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn unfinished_send_matches_exact_file_only() {
        let store = Store::open_in_memory().unwrap();
        let desk = device("Desk").id;
        let t = new_transfer(&desk, Direction::Sent);
        store.record_transfer_started(&t).unwrap();
        let path = t.path.clone().unwrap();

        let found = store
            .unfinished_send(&desk, &path, t.file_size, t.source_modified)
            .unwrap();
        assert_eq!(found, Some(t.id));
        // Changed size, changed time, other device: not resumable
        assert_eq!(
            store
                .unfinished_send(&desk, &path, 999, t.source_modified)
                .unwrap(),
            None
        );
        let later = t.source_modified.map(|m| m + Duration::from_secs(1));
        assert_eq!(
            store
                .unfinished_send(&desk, &path, t.file_size, later)
                .unwrap(),
            None
        );
        assert_eq!(
            store
                .unfinished_send(&device("Other").id, &path, t.file_size, t.source_modified)
                .unwrap(),
            None
        );

        store
            .record_transfer_completed(&t.id, None, &blake3::hash(b"x"))
            .unwrap();
        assert_eq!(
            store
                .unfinished_send(&desk, &path, t.file_size, t.source_modified)
                .unwrap(),
            None
        );
    }

    #[test]
    fn settings_round_trip() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.setting("receive_dir").unwrap(), None);
        store.set_setting("receive_dir", "D:/TOVI").unwrap();
        store.set_setting("receive_dir", "E:/TOVI").unwrap();
        assert_eq!(
            store.setting("receive_dir").unwrap().as_deref(),
            Some("E:/TOVI")
        );
    }

    #[test]
    fn check_constraints_reject_bad_rows() {
        let store = Store::open_in_memory().unwrap();
        let bad = store.with_conn(|c| {
            c.execute(
                "INSERT INTO devices (id, name, platform, paired_at) VALUES (x'00', 'x', 'y', 0)",
                [],
            )
        });
        assert!(bad.is_err());
    }
}
