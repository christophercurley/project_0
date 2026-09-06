use crate::{
    error::{ApiError, Result},
    security,
};
use daymark_domain::{AuditTime, UserId};
use daymark_persistence::Store;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};

const MIGRATION: &str = include_str!("../migrations/001_identity.sql");
pub const SESSION_SECONDS: i64 = 43_200;
pub const IDLE_SECONDS: i64 = 1_800;

pub(crate) struct Db {
    pub store: Store,
    pub identity: Connection,
    // Never unlink this file: replacing it would let a second process lock a new inode.
    _lock: File,
}
#[derive(Clone)]
pub(crate) struct Session {
    pub token: String,
    pub csrf: Option<String>,
}
pub(crate) struct Principal {
    pub owner: UserId,
    pub username: String,
    pub admin: bool,
    pub csrf: String,
}
impl Principal {
    pub fn admin(&self) -> Result<()> {
        if self.admin {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }
    pub fn view(&self) -> Value {
        json!({"id":self.owner.0.to_string(),"username":self.username,"admin":self.admin,"csrf":self.csrf})
    }
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        // Canonical directory + filename makes relative spellings share a lock.
        // Operator must not use hard-link aliases or remove locks while running.
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = parent.canonicalize().map_err(|_| ApiError::busy())?;
        let name = path.file_name().ok_or_else(ApiError::bad)?;
        let candidate = parent.join(name);
        let path = match candidate.symlink_metadata() {
            // This also rejects dangling symlinks: SQLite must not create a
            // target after we have locked a different, unresolved alias path.
            Ok(_) => candidate.canonicalize().map_err(|_| ApiError::busy())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => candidate,
            Err(_) => return Err(ApiError::busy()),
        };
        let mut lock_name = path.as_os_str().to_os_string();
        lock_name.push(".application.lock");
        let lock_path = std::path::PathBuf::from(lock_name);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(|_| ApiError::busy())?;
        lock.try_lock().map_err(|_| ApiError::busy())?;
        let store = Store::open(&path)?;
        let mut identity = Connection::open(&path)?;
        identity.busy_timeout(Duration::from_secs(5))?;
        identity.pragma_update(None, "foreign_keys", true)?;
        identity.pragma_update(None, "synchronous", "FULL")?;
        let tx = identity.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS identity_migrations(version INTEGER PRIMARY KEY, sql TEXT NOT NULL) STRICT")?;
        let applied: Vec<(i64, String)> = tx
            .prepare("SELECT version,sql FROM identity_migrations ORDER BY version")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let migration = MIGRATION.replace("\r\n", "\n");
        match applied.as_slice() {
            [] => {
                // M2 was a library test milestone. Never assign unclaimed source
                // owners to newly registered accounts in an existing nonempty DB.
                let count: i64 = tx.query_row("SELECT count(*) FROM sources", [], |r| r.get(0))?;
                if count != 0 {
                    return Err(ApiError::busy());
                }
                tx.execute_batch(MIGRATION)?;
                tx.execute("INSERT INTO identity_migrations VALUES (1,?1)", [migration])?;
            }
            [(1, sql)] if sql.replace("\r\n", "\n") == migration => {}
            _ => return Err(ApiError::busy()),
        }
        tx.commit()?;
        Ok(Self {
            store,
            identity,
            _lock: lock,
        })
    }

    pub fn account(&mut self, username: &str, hash: &str, admin: bool, now: i64) -> Result<Value> {
        let tx = self
            .identity
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if admin
            && tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE admin=1)",
                [],
                |r| r.get::<_, bool>(0),
            )?
        {
            return Err(ApiError::conflict());
        }
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE username=?1)",
            [username],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(ApiError::conflict());
        }
        let id = security::random::<8>()?;
        tx.execute("INSERT INTO accounts(id,username,password_hash,admin,created_at) VALUES (?1,?2,?3,?4,?5)",params![id,username,hash,admin,now])?;
        tx.execute(
            "INSERT INTO account_history(actor,target,action,at) VALUES (?1,?1,?2,?3)",
            params![id, if admin { "bootstrapped" } else { "registered" }, now],
        )?;
        tx.commit()?;
        Ok(json!({"id":u64::from_be_bytes(id).to_string(),"username":username,"admin":admin}))
    }

    pub fn credentials(&self, username: &str) -> Result<Option<String>> {
        Ok(self
            .identity
            .query_row(
                "SELECT password_hash FROM accounts WHERE username=?1",
                [username],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn login(
        &mut self,
        username: &str,
        verified_hash: &str,
        old: Option<Session>,
        now: i64,
    ) -> Result<(String, Value)> {
        let tx = self
            .identity
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Password verification runs outside this worker. Recheck the exact hash
        // under the write transaction so a concurrent reset cannot revive it.
        let row: Option<(Vec<u8>, bool)> = tx
            .query_row(
                "SELECT id,admin FROM accounts WHERE username=?1 AND password_hash=?2",
                params![username, verified_hash],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (id, admin) = row.ok_or_else(ApiError::unauthorized)?;
        tx.execute(
            "DELETE FROM sessions WHERE expires_at<=?1 OR last_seen<=?2",
            params![now, now - IDLE_SECONDS],
        )?;
        if let Some(old) = old {
            tx.execute(
                "DELETE FROM sessions WHERE token_hash=?1",
                [security::digest(&old.token)],
            )?;
        }
        tx.execute("DELETE FROM sessions WHERE token_hash IN (SELECT token_hash FROM sessions WHERE owner=?1 ORDER BY last_seen DESC,token_hash LIMIT -1 OFFSET 4)",[&id])?;
        let token = security::token()?;
        let csrf = security::token()?;
        tx.execute(
            "INSERT INTO sessions VALUES (?1,?2,?3,?4,?5)",
            params![
                security::digest(&token),
                id,
                csrf,
                now + SESSION_SECONDS,
                now
            ],
        )?;
        tx.commit()?;
        let principal = Principal {
            owner: owner(id)?,
            username: username.into(),
            admin,
            csrf,
        };
        Ok((token, principal.view()))
    }

    pub fn authenticate(
        &mut self,
        session: &Session,
        mutation: bool,
        now: i64,
    ) -> Result<Principal> {
        let row: Option<(Vec<u8>,String,bool,String)> = self.identity.query_row("SELECT a.id,a.username,a.admin,s.csrf FROM sessions s JOIN accounts a ON a.id=s.owner WHERE s.token_hash=?1 AND s.expires_at>?2 AND s.last_seen>?3",params![security::digest(&session.token),now,now-IDLE_SECONDS],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let (id, username, admin, csrf) = row.ok_or_else(ApiError::unauthorized)?;
        if mutation
            && session
                .csrf
                .as_ref()
                .is_none_or(|value| !security::equal_secret(value, &csrf))
        {
            return Err(ApiError::forbidden());
        }
        self.identity.execute(
            "UPDATE sessions SET last_seen=max(last_seen,?1) WHERE token_hash=?2",
            params![now, security::digest(&session.token)],
        )?;
        Ok(Principal {
            owner: owner(id)?,
            username,
            admin,
            csrf,
        })
    }
    pub fn logout(&mut self, session: &Session) -> Result<()> {
        self.identity.execute(
            "DELETE FROM sessions WHERE token_hash=?1",
            [security::digest(&session.token)],
        )?;
        Ok(())
    }
    pub fn reset(&mut self, actor: UserId, target: UserId, hash: &str, now: i64) -> Result<()> {
        let tx = self
            .identity
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE accounts SET password_hash=?1 WHERE id=?2 AND admin=0",
            params![hash, target.0.to_be_bytes()],
        )?;
        if changed != 1 {
            return Err(ApiError::missing());
        }
        tx.execute(
            "DELETE FROM sessions WHERE owner=?1",
            [target.0.to_be_bytes()],
        )?;
        tx.execute("INSERT INTO account_history(actor,target,action,at) VALUES (?1,?2,'password_reset',?3)",params![actor.0.to_be_bytes(),target.0.to_be_bytes(),now])?;
        tx.commit()?;
        Ok(())
    }
    pub fn settings(&self, owner: UserId) -> Result<Value> {
        let text: String = self.identity.query_row(
            "SELECT settings FROM accounts WHERE id=?1",
            [owner.0.to_be_bytes()],
            |r| r.get(0),
        )?;
        serde_json::from_str(&text).map_err(|_| ApiError::busy())
    }
    pub fn set_settings(&mut self, owner: UserId, value: Value, now: i64) -> Result<Value> {
        let tx = self
            .identity
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE accounts SET settings=?1 WHERE id=?2",
            params![value.to_string(), owner.0.to_be_bytes()],
        )?;
        tx.execute("INSERT INTO account_history(actor,target,action,at) VALUES (?1,?1,'settings_changed',?2)",params![owner.0.to_be_bytes(),now])?;
        tx.commit()?;
        Ok(value)
    }
    pub fn accounts(&self, offset: u32, limit: u32) -> Result<Value> {
        let mut stmt = self.identity.prepare("SELECT id,username,admin,created_at FROM accounts ORDER BY username LIMIT ?1 OFFSET ?2")?;
        let rows = stmt.query_map(params![limit, offset], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, bool>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, username, admin, created) = row?;
            out.push(json!({"id":owner(id)?.0.to_string(),"username":username,"admin":admin,"created_at":created}));
        }
        Ok(json!(out))
    }
    pub fn history(&self, owner: UserId, id: u64, offset: u32, limit: u32) -> Result<Value> {
        // Page before decoding: audit snapshots can be large. Same owned SQL
        // contract and stored Change format as Store::history, without loading
        // every revision merely to return a bounded page.
        let mut stmt = self.identity.prepare("SELECT payload FROM source_history WHERE owner=?1 AND source_id=?2 ORDER BY sequence LIMIT ?3 OFFSET ?4")?;
        let rows = stmt.query_map(
            params![owner.0.to_be_bytes(), id.to_be_bytes(), limit, offset],
            |r| r.get::<_, String>(0),
        )?;
        let mut out = Vec::new();
        for row in rows {
            let change: daymark_domain::Change =
                serde_json::from_str(&row?).map_err(|_| ApiError::busy())?;
            out.push(crate::wire::output(change)?);
        }
        Ok(json!(out))
    }

    pub fn throttle(
        &mut self,
        registration: bool,
        username: &str,
        address: &str,
        now: i64,
    ) -> Result<()> {
        let tx = self
            .identity
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM auth_limits WHERE started<=?1", [now - 3600])?;
        let kind = if registration { "register" } else { "login" };
        let keys = [
            ("global".into(), 120, 600),
            (
                format!("{kind}:ip:{}", security::hex(&security::digest(address))),
                if registration { 5 } else { 20 },
                if registration { 3600 } else { 600 },
            ),
            (
                format!("{kind}:user:{}", security::hex(&security::digest(username))),
                10,
                600,
            ),
        ];
        let mut allowed = true;
        for (key, max, seconds) in keys {
            let count: i64 = tx.query_row("SELECT count(*) FROM auth_limits", [], |r| r.get(0))?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM auth_limits WHERE key=?1)",
                [&key],
                |r| r.get(0),
            )?;
            if count >= 4096 && !exists {
                allowed = false;
                continue;
            }
            tx.execute("INSERT INTO auth_limits VALUES (?1,?2,1) ON CONFLICT(key) DO UPDATE SET attempts=CASE WHEN started<=?3 THEN 1 ELSE min(attempts+1,1000000) END,started=CASE WHEN started<=?3 THEN ?2 ELSE started END",params![key,now,now-seconds])?;
            let attempts: i64 = tx.query_row(
                "SELECT attempts FROM auth_limits WHERE key=?1",
                [key],
                |r| r.get(0),
            )?;
            allowed &= attempts <= max;
        }
        tx.commit()?;
        if allowed {
            Ok(())
        } else {
            Err(ApiError::limited())
        }
    }
    pub fn time(now: i64) -> AuditTime {
        AuditTime(now)
    }
}
fn owner(bytes: Vec<u8>) -> Result<UserId> {
    Ok(UserId(u64::from_be_bytes(
        bytes.try_into().map_err(|_| ApiError::busy())?,
    )))
}
