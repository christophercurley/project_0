//! Synchronous SQLite transaction adapter. The application supplies trusted owner
//! IDs, authorized calendar actors, and UTC times. No authentication is provided.
//! Run blocking work on a bounded blocking executor when adding Tokio in M3.

use daymark_domain::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use std::{fmt, path::Path, time::Duration};

const MIGRATION: &str = include_str!("../migrations/001_initial.sql");

#[derive(Debug)]
pub enum StoreError {
    Domain(Error),
    Storage(rusqlite::Error),
    Encoding(serde_json::Error),
    StaleCalendar,
    IncompatibleSchema,
    CorruptState,
}
impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(e) => e.fmt(f),
            Self::StaleCalendar => {
                write!(f, "The holiday calendar changed; reload before submitting.")
            }
            Self::IncompatibleSchema => {
                write!(f, "Unsupported or modified database migration history.")
            }
            Self::CorruptState => write!(f, "Stored state is inconsistent."),
            Self::Storage(_) => write!(f, "The database operation failed; no change was accepted."),
            Self::Encoding(_) => write!(f, "Stored data could not be encoded or decoded."),
        }
    }
}
impl std::error::Error for StoreError {}
impl From<Error> for StoreError {
    fn from(e: Error) -> Self {
        Self::Domain(e)
    }
}
impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e)
    }
}
impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::Encoding(e)
    }
}
pub type Result<T> = std::result::Result<T, StoreError>;

/// One connection, never a cached in-memory ledger. Independent instances/processes
/// coordinate through SQLite. Personal methods always require an owner; current
/// record lookups and edits/deletes also require the expected year.
pub struct Store {
    connection: Connection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedCalendar {
    pub revision: i64,
    pub calendar: Calendar,
}

impl Store {
    /// Opens/initializes a local SQLite file. Caller owns its persistent directory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, sql TEXT NOT NULL) STRICT;")?;
        let applied = {
            let mut stmt =
                tx.prepare("SELECT version, sql FROM schema_migrations ORDER BY version")?;
            stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        // Checkouts may use CRLF on Windows and LF on Linux. Database portability
        // must not depend on the build host's Git line-ending setting.
        let migration = MIGRATION.replace("\r\n", "\n");
        match applied.as_slice() {
            [] => {
                tx.execute_batch(MIGRATION)?;
                tx.execute("INSERT INTO schema_migrations VALUES (1, ?1)", [&migration])?;
            }
            [(1, sql)] if sql.replace("\r\n", "\n") == migration => {}
            _ => return Err(StoreError::IncompatibleSchema),
        }
        tx.commit()?;
        Ok(Self { connection })
    }

    pub fn calendar(&self, year: Year) -> Result<Option<VersionedCalendar>> {
        calendar(&self.connection, year)
    }

    /// Privileged application boundary; expected None means never configured.
    pub fn configure(
        &mut self,
        actor: UserId,
        at: AuditTime,
        expected: Option<i64>,
        next: Calendar,
    ) -> Result<i64> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = calendar(&tx, next.year)?;
        if old.as_ref().map(|c| c.revision) != expected {
            return Err(StoreError::StaleCalendar);
        }
        let revision = expected
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(Error::Overflow)?;
        // Only this internal privileged path reads all owners, only in this year.
        // The domain checks every active reference before permitting correction.
        let mut ledger = load(&tx, Scope::Calendar(next.year))?;
        ledger.configure(actor, at, next.clone())?;
        tx.execute("INSERT INTO calendars(year,revision,payload) VALUES (?1,?2,?3) ON CONFLICT(year) DO UPDATE SET revision=excluded.revision,payload=excluded.payload",
            params![next.year.get(), revision, encode(&next)?])?;
        tx.execute("DELETE FROM holidays WHERE year=?1", [next.year.get()])?;
        for holiday in &next.holidays {
            tx.execute(
                "INSERT INTO holidays(year,id,date,name) VALUES (?1,?2,?3,?4)",
                params![
                    next.year.get(),
                    key(holiday.id.0),
                    holiday.date.to_string(),
                    holiday.name
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO calendar_history(year,revision,at,payload) VALUES (?1,?2,?3,?4)",
            params![
                next.year.get(),
                revision,
                at.0,
                encode(&ledger.calendar_history()[0])?
            ],
        )?;
        tx.commit()?;
        Ok(revision)
    }

    pub fn create(
        &mut self,
        owner: UserId,
        at: AuditTime,
        id: EventId,
        source: Source,
    ) -> Result<Change> {
        self.mutate(owner, |ledger| ledger.create(owner, at, id, source))
    }
    /// `year` is the expected current year, including for an edit moving years.
    pub fn edit(
        &mut self,
        owner: UserId,
        year: Year,
        at: AuditTime,
        id: EventId,
        revision: u64,
        source: Source,
    ) -> Result<Change> {
        self.mutate(owner, |ledger| {
            check_year(ledger, owner, year, id)?;
            ledger.edit(owner, at, id, revision, source)
        })
    }
    pub fn delete(
        &mut self,
        owner: UserId,
        year: Year,
        at: AuditTime,
        id: EventId,
        revision: u64,
    ) -> Result<Change> {
        self.mutate(owner, |ledger| {
            check_year(ledger, owner, year, id)?;
            ledger.delete(owner, at, id, revision)
        })
    }

    fn mutate(
        &mut self,
        owner: UserId,
        apply: impl FnOnce(&mut Ledger) -> std::result::Result<(), Error>,
    ) -> Result<Change> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut ledger = load(&tx, Scope::Owner(owner))?;
        apply(&mut ledger)?;
        let change = &ledger.history(owner)[0];
        let record = change
            .after
            .as_ref()
            .or(change.before.as_ref())
            .ok_or(StoreError::CorruptState)?;
        let holiday = change.after.as_ref().and_then(|r| match r.source.activity {
            Activity::HolidayUse { holiday } | Activity::HolidayWork { holiday, .. } => {
                Some(key(holiday.0))
            }
            _ => None,
        });
        tx.execute("INSERT INTO sources(owner,id,revision,year,holiday_id,payload) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(owner,id) DO UPDATE SET revision=excluded.revision,year=excluded.year,holiday_id=excluded.holiday_id,payload=excluded.payload",
            params![key(owner.0), key(change.id.0), key(record.reference.revision), record.source.dates[0].year().get(), holiday, change.after.as_ref().map(encode).transpose()?])?;
        for snapshot in &change.after_effects {
            tx.execute(
                "DELETE FROM effects WHERE owner=?1 AND year=?2",
                params![key(owner.0), snapshot.year.get()],
            )?;
            for (ordinal, effect) in snapshot
                .effects
                .iter()
                .filter(|e| !matches!(e.origin, Origin::Entitlement { .. }))
                .enumerate()
            {
                let ordinal = i64::try_from(ordinal).map_err(|_| Error::Overflow)?;
                tx.execute(
                    "INSERT INTO effects(owner,year,ordinal,payload) VALUES (?1,?2,?3,?4)",
                    params![key(owner.0), snapshot.year.get(), ordinal, encode(effect)?],
                )?;
                let supports = match &effect.origin {
                    Origin::Source(reference) => std::slice::from_ref(reference),
                    Origin::WorkedHoliday { supports, .. } => supports.as_slice(),
                    Origin::Entitlement { .. } => unreachable!("filtered above"),
                };
                for reference in supports {
                    tx.execute("INSERT INTO effect_supports(owner,year,ordinal,source_id,revision) VALUES (?1,?2,?3,?4,?5)",
                        params![key(owner.0), snapshot.year.get(), ordinal, key(reference.id.0), key(reference.revision)])?;
                }
            }
        }
        tx.execute(
            "INSERT INTO source_history(owner,source_id,at,payload) VALUES (?1,?2,?3,?4)",
            params![key(owner.0), key(change.id.0), change.at.0, encode(change)?],
        )?;
        tx.commit()?;
        Ok(change.clone())
    }

    pub fn record(&self, owner: UserId, year: Year, id: EventId) -> Result<Record> {
        let payload: Option<String> = self.connection.query_row(
            "SELECT payload FROM sources WHERE owner=?1 AND year=?2 AND id=?3 AND payload IS NOT NULL",
            params![key(owner.0), year.get(), key(id.0)], |r| r.get(0)).optional()?;
        decode(&payload.ok_or(Error::NotFound)?)
    }

    pub fn snapshot(&mut self, owner: UserId, year: Year) -> Result<Snapshot> {
        let tx = self.connection.transaction()?;
        let ledger = load(&tx, Scope::OwnerYear(owner, year))?;
        let snapshot = ledger.snapshot(owner, year)?;
        verify_effects(&tx, owner, &snapshot)?;
        tx.commit()?;
        Ok(snapshot)
    }

    pub fn query(&mut self, owner: UserId, year: Year, filter: &Filter) -> Result<Vec<Effect>> {
        let tx = self.connection.transaction()?;
        let ledger = load(&tx, Scope::OwnerYear(owner, year))?;
        verify_effects(&tx, owner, &ledger.snapshot(owner, year)?)?;
        let effects = ledger.query(owner, year, filter)?;
        tx.commit()?;
        Ok(effects)
    }

    /// Owned source history includes both years of a deliberate year-moving edit.
    pub fn history(&self, owner: UserId, id: EventId) -> Result<Vec<Change>> {
        let mut stmt = self.connection.prepare(
            "SELECT payload FROM source_history WHERE owner=?1 AND source_id=?2 ORDER BY sequence",
        )?;
        let rows = stmt.query_map(params![key(owner.0), key(id.0)], |r| r.get::<_, String>(0))?;
        rows.map(|r| decode(&r?)).collect()
    }

    /// Global configuration audit; exposes no personal source data.
    pub fn calendar_history(&self, year: Year) -> Result<Vec<CalendarChange>> {
        let mut stmt = self
            .connection
            .prepare("SELECT payload FROM calendar_history WHERE year=?1 ORDER BY revision")?;
        let rows = stmt.query_map([year.get()], |r| r.get::<_, String>(0))?;
        rows.map(|r| decode(&r?)).collect()
    }
}

fn check_year(
    ledger: &Ledger,
    owner: UserId,
    year: Year,
    id: EventId,
) -> std::result::Result<(), Error> {
    if ledger.record(owner, id)?.source.dates[0].year() != year {
        return Err(Error::NotFound);
    }
    Ok(())
}
fn encode(value: &impl Serialize) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
fn decode<T: DeserializeOwned>(value: &str) -> Result<T> {
    Ok(serde_json::from_str(value)?)
}
fn key(value: u64) -> [u8; 8] {
    value.to_be_bytes()
}
fn id(value: Vec<u8>) -> Result<u64> {
    Ok(u64::from_be_bytes(
        value.try_into().map_err(|_| StoreError::CorruptState)?,
    ))
}
fn calendar(conn: &Connection, year: Year) -> Result<Option<VersionedCalendar>> {
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT revision,payload FROM calendars WHERE year=?1",
            [year.get()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(revision, payload)| {
        Ok(VersionedCalendar {
            revision,
            calendar: decode(&payload)?,
        })
    })
    .transpose()
}

enum Scope {
    Owner(UserId),
    OwnerYear(UserId, Year),
    Calendar(Year),
}
fn load(conn: &Connection, scope: Scope) -> Result<Ledger> {
    let calendars = {
        let mut stmt = conn.prepare("SELECT payload FROM calendars ORDER BY year")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .map(|r| decode(&r?))
            .collect::<Result<Vec<Calendar>>>()?
    };
    let (sql, owner, year) = match scope {
        Scope::Owner(owner) => (
            "SELECT owner,id,revision,year,payload FROM sources WHERE owner=?1",
            Some(key(owner.0)),
            None,
        ),
        Scope::OwnerYear(owner, year) => (
            "SELECT owner,id,revision,year,payload FROM sources WHERE owner=?1 AND year=?2",
            Some(key(owner.0)),
            Some(year.get()),
        ),
        Scope::Calendar(year) => (
            "SELECT owner,id,revision,year,payload FROM sources WHERE year=?2 AND payload IS NOT NULL AND ?1 IS NULL",
            None,
            Some(year.get()),
        ),
    };
    let mut stmt = conn.prepare(sql)?;
    let mut rows = match scope {
        Scope::Owner(_) => stmt.query(params![owner])?,
        _ => stmt.query(params![owner, year])?,
    };
    let mut records = Vec::new();
    let mut used = Vec::new();
    while let Some(row) = rows.next()? {
        let owner = UserId(id(row.get(0)?)?);
        let event = EventId(id(row.get(1)?)?);
        used.push((owner, event));
        if let Some(payload) = row.get::<_, Option<String>>(4)? {
            let record: Record = decode(&payload)?;
            if record.reference.id != event
                || record.reference.revision != id(row.get(2)?)?
                || record.source.dates.first().map(|d| d.year().get()) != Some(row.get(3)?)
            {
                return Err(StoreError::CorruptState);
            }
            records.push((owner, record));
        }
    }
    Ok(Ledger::restore(calendars, records, used)?)
}

fn verify_effects(conn: &Connection, owner: UserId, snapshot: &Snapshot) -> Result<()> {
    let mut stmt =
        conn.prepare("SELECT payload FROM effects WHERE owner=?1 AND year=?2 ORDER BY ordinal")?;
    let stored = stmt
        .query_map(params![key(owner.0), snapshot.year.get()], |r| {
            r.get::<_, String>(0)
        })?
        .map(|r| decode(&r?))
        .collect::<Result<Vec<Effect>>>()?;
    let expected: Vec<_> = snapshot
        .effects
        .iter()
        .filter(|e| !matches!(e.origin, Origin::Entitlement { .. }))
        .cloned()
        .collect();
    if stored != expected {
        return Err(StoreError::CorruptState);
    }
    Ok(())
}
