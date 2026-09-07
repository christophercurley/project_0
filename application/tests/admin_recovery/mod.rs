use super::*;
#[path = "../common/mod.rs"]
mod common;
use crate::{App, Config};
use common::*;
use rusqlite::{Connection, types::Value as SqlValue};
use serde_json::{Value, json};
mod review;

const NEW: &str = "replacement synthetic password 42";
struct Answers {
    terminal: bool,
    values: std::collections::VecDeque<String>,
}
impl Answers {
    fn new(first: &str, second: &str) -> Self {
        Self {
            terminal: true,
            values: [first.into(), second.into()].into(),
        }
    }
}
impl Prompt for Answers {
    fn interactive(&self) -> bool {
        self.terminal
    }
    fn password(&mut self, _: &str) -> Result<Zeroizing<String>> {
        self.values
            .pop_front()
            .map(Zeroizing::new)
            .ok_or_else(ApiError::bad)
    }
}
fn dump(raw: &Connection, table: &str) -> Vec<Vec<SqlValue>> {
    let mut stmt = raw
        .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
        .unwrap();
    let columns = stmt.column_count();
    stmt.query_map([], |r| {
        (0..columns)
            .map(|i| r.get(i))
            .collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}
fn all(raw: &Connection) -> Vec<Vec<Vec<SqlValue>>> {
    [
        "accounts",
        "sessions",
        "account_history",
        "admin_recovery_history",
        "sources",
        "effects",
        "effect_supports",
        "source_history",
        "calendars",
        "calendar_history",
        "auth_limits",
        "sqlite_sequence",
    ]
    .map(|t| dump(raw, t))
    .into()
}
async fn recover(path: std::path::PathBuf, first: &str, second: &str) -> Result<()> {
    let mut input = Answers::new(first, second);
    tokio::task::spawn_blocking(move || recover_with_prompt(&path, &mut input))
        .await
        .unwrap()
}

#[tokio::test]
async fn sole_admin_recovery_revokes_all_old_authority_preserves_ordinary_data_and_survives_reopen()
{
    let f = Fixture::new().await;
    let user = f.user("alice").await;
    let admin = f.admin().await;
    let other_admin_session = login(&f.app, "operator", PASSWORD, None).await;
    assert_eq!(
        create(&f.app, &user, "1", 8, "private ordinary ledger")
            .await
            .status,
        201
    );
    assert_eq!(
        send(
            &f.app,
            "PUT",
            "/api/v1/settings",
            Some(&user),
            json!({"hire_date":"2000-01-01","annual_pto_allowance":"160"})
        )
        .await
        .status,
        200
    );
    let raw = f.raw();
    let original_accounts = dump(&raw, "accounts");
    let ordinary_key = user.id.parse::<u64>().unwrap().to_be_bytes();
    let ordinary_sessions: Vec<_> = dump(&raw, "sessions")
        .into_iter()
        .filter(|row| row[1] == SqlValue::Blob(ordinary_key.to_vec()))
        .collect();
    let preserved = [
        "account_history",
        "sources",
        "effects",
        "effect_supports",
        "source_history",
        "calendars",
        "calendar_history",
        "auth_limits",
    ]
    .map(|t| dump(&raw, t));
    let Fixture { app, dir, clock } = f;
    drop(app);
    recover(dir.path().join("test.sqlite"), NEW, NEW)
        .await
        .unwrap();
    let changed = dump(&raw, "accounts");
    for (old, new) in original_accounts.iter().zip(&changed) {
        if old[1] == SqlValue::Text("alice".into()) {
            assert_eq!(old, new);
        } else {
            for i in [0, 1, 3, 4, 5] {
                assert_eq!(old[i], new[i]);
            }
            assert_ne!(old[2], new[2]);
            let SqlValue::Text(hash) = &new[2] else {
                panic!("missing hash")
            };
            assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        }
    }
    for (table, expected) in [
        "account_history",
        "sources",
        "effects",
        "effect_supports",
        "source_history",
        "calendars",
        "calendar_history",
        "auth_limits",
    ]
    .iter()
    .zip(preserved)
    {
        assert_eq!(dump(&raw, table), expected);
    }
    let sessions = dump(&raw, "sessions");
    assert_eq!(sessions, ordinary_sessions);
    let audit = dump(&raw, "admin_recovery_history");
    assert_eq!(audit.len(), 1);
    assert_eq!(
        audit[0][1],
        SqlValue::Blob(admin.id.parse::<u64>().unwrap().to_be_bytes().to_vec())
    );
    assert_eq!(audit[0][2], SqlValue::Text("local_operator".into()));
    assert!(matches!(audit[0][3],SqlValue::Integer(n) if n>0));
    let text = format!("{audit:?}");
    for secret in [
        PASSWORD,
        NEW,
        &admin.cookie,
        &admin.csrf,
        &other_admin_session.cookie,
    ] {
        assert!(!text.contains(secret));
    }
    assert!(
        raw.execute("UPDATE admin_recovery_history SET at=0", [])
            .is_err()
    );
    assert!(
        raw.execute("DELETE FROM admin_recovery_history", [])
            .is_err()
    );
    let app = App::with_clock(
        dir.path().join("test.sqlite"),
        Config::https(ORIGIN).unwrap(),
        clock.clone(),
    )
    .await
    .unwrap();
    for stale in [&admin, &other_admin_session] {
        assert_eq!(
            send(&app, "GET", "/api/v1/session", Some(stale), Value::Null)
                .await
                .status,
            401
        );
        assert_eq!(
            send(
                &app,
                "PUT",
                "/api/v1/years/2026/holidays",
                Some(stale),
                calendar(Value::Null)
            )
            .await
            .status,
            401
        );
        assert_eq!(
            send(
                &app,
                "POST",
                "/api/v1/login",
                Some(stale),
                json!({"username":"operator","password":PASSWORD})
            )
            .await
            .status,
            401
        );
    }
    assert_eq!(
        send(&app, "GET", "/api/v1/session", Some(&user), Value::Null)
            .await
            .status,
        200
    );
    login(&app, "alice", PASSWORD, None).await;
    let fresh = login(&app, "operator", NEW, None).await;
    assert_ne!(fresh.cookie, admin.cookie);
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/v1/admin/accounts",
            Some(&fresh),
            Value::Null
        )
        .await
        .status,
        200
    );
    // A subsequent password login may issue new authority, never revive a token.
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/v1/admin/accounts",
            Some(&admin),
            Value::Null
        )
        .await
        .status,
        401
    );
    drop(app);
    let app = App::with_clock(
        dir.path().join("test.sqlite"),
        Config::https(ORIGIN).unwrap(),
        clock,
    )
    .await
    .unwrap();
    assert_eq!(
        send(&app, "GET", "/api/v1/session", Some(&admin), Value::Null)
            .await
            .status,
        401
    );
    login(&app, "operator", NEW, None).await;
}

#[tokio::test]
async fn invalid_mismatched_noninteractive_and_missing_prompt_input_leave_state_unchanged() {
    let f = Fixture::new().await;
    f.admin().await;
    let raw = f.raw();
    let before = all(&raw);
    let Fixture { app, dir, .. } = f;
    drop(app);
    for (first, second) in [
        (NEW, PASSWORD),
        ("short", "short"),
        ("", ""),
        (&"x".repeat(129), &"x".repeat(129)),
    ] {
        assert!(
            recover(dir.path().join("test.sqlite"), first, second)
                .await
                .is_err()
        );
        assert_eq!(all(&raw), before);
    }
    let mut prompt = Answers::new(NEW, NEW);
    prompt.terminal = false;
    assert!(recover_with_prompt(&dir.path().join("test.sqlite"), &mut prompt).is_err());
    assert_eq!(prompt.values.len(), 2);
    let mut prompt = Answers::new(NEW, NEW);
    prompt.values.pop_back();
    assert!(recover_with_prompt(&dir.path().join("test.sqlite"), &mut prompt).is_err());
    assert_eq!(all(&raw), before);
    assert!(
        recover(dir.path().join("missing.sqlite"), NEW, NEW)
            .await
            .is_err()
    );
    assert!(!dir.path().join("missing.sqlite").exists());
}

#[tokio::test]
async fn zero_or_multiple_administrators_refuse_recovery_without_targeting_ordinary_accounts() {
    for multiple in [false, true] {
        let f = Fixture::new().await;
        f.user("alice").await;
        if multiple {
            f.admin().await;
        }
        let raw = f.raw();
        if multiple {
            raw.execute_batch("DROP INDEX one_initial_admin; INSERT INTO accounts(id,username,password_hash,admin,created_at) VALUES(x'0000000000000012','unexpected','synthetic',1,0)").unwrap();
        }
        let before = all(&raw);
        let Fixture { app, dir, .. } = f;
        drop(app);
        assert!(
            recover(dir.path().join("test.sqlite"), NEW, NEW)
                .await
                .is_err()
        );
        assert_eq!(all(&raw), before);
    }
}

#[tokio::test]
async fn failures_at_revocation_audit_and_commit_roll_back_the_entire_recovery() {
    for fault in [
        "CREATE TRIGGER injected BEFORE DELETE ON sessions BEGIN SELECT RAISE(ABORT,'sensitive failure'); END;",
        "CREATE TRIGGER injected BEFORE INSERT ON admin_recovery_history BEGIN SELECT RAISE(ABORT,'sensitive failure'); END;",
        "CREATE TABLE fail_commit(id BLOB REFERENCES accounts(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER injected AFTER INSERT ON admin_recovery_history BEGIN INSERT INTO fail_commit VALUES(x'0000000000000011'); END;",
    ] {
        let f = Fixture::new().await;
        f.user("alice").await;
        f.admin().await;
        let raw = f.raw();
        raw.execute_batch(fault).unwrap();
        let before = all(&raw);
        let Fixture { app, dir, .. } = f;
        drop(app);
        let error = recover(dir.path().join("test.sqlite"), NEW, NEW)
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("sensitive"));
        assert!(!error.to_string().contains(NEW));
        assert_eq!(all(&raw), before);
        raw.execute_batch("DROP TRIGGER injected").unwrap();
        recover(dir.path().join("test.sqlite"), NEW, NEW)
            .await
            .unwrap();
        assert_eq!(dump(&raw, "admin_recovery_history").len(), 1);
    }
}

#[tokio::test]
async fn recovery_uses_same_lock_as_live_application_and_operator() {
    let f = Fixture::new().await;
    f.admin().await;
    let raw = f.raw();
    let before = all(&raw);
    for path in [
        f.dir.path().join("test.sqlite"),
        f.dir.path().join(".").join("test.sqlite"),
        f.dir.path().join("test.sqlite").canonicalize().unwrap(),
    ] {
        assert!(recover(path, NEW, NEW).await.is_err());
        assert_eq!(all(&raw), before);
    }
    let Fixture { app, dir, .. } = f;
    drop(app);
    let operator = Db::open(&dir.path().join("test.sqlite")).unwrap();
    assert!(
        recover(dir.path().join("test.sqlite"), NEW, NEW)
            .await
            .is_err()
    );
    assert_eq!(all(&raw), before);
    drop(operator);
    recover(dir.path().join("test.sqlite"), NEW, NEW)
        .await
        .unwrap();
}

#[test]
fn subprocess_recovery_worker() {
    let Some(path) = std::env::var_os("DAYMARK_RECOVERY_TEST_DB") else {
        return;
    };
    let result = recover_with_prompt(Path::new(&path), &mut Answers::new(NEW, NEW));
    assert_eq!(
        result.is_ok(),
        std::env::var_os("DAYMARK_RECOVERY_TEST_SUCCESS").is_some()
    );
}

fn run_recovery_process(path: &Path, success: bool) {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "operator::admin_recovery::subprocess_recovery_worker",
        ])
        .env("DAYMARK_RECOVERY_TEST_DB", path)
        .env_remove("DAYMARK_RECOVERY_TEST_SUCCESS")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if success {
        command.env("DAYMARK_RECOVERY_TEST_SUCCESS", "1");
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "recovery subprocess failed");
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("recovery subprocess timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test]
async fn recovery_cannot_cross_another_process_application_or_operator_lock() {
    let f = Fixture::new().await;
    f.admin().await;
    let raw = f.raw();
    let before = all(&raw);
    let path = f.dir.path().join(".").join("test.sqlite");
    run_recovery_process(&path, false);
    assert_eq!(all(&raw), before);
    let Fixture { app, dir, .. } = f;
    drop(app);
    let operator = Db::open(&dir.path().join("test.sqlite")).unwrap();
    run_recovery_process(&path, false);
    assert_eq!(all(&raw), before);
    drop(operator);
    run_recovery_process(&path, true);
    assert_ne!(dump(&raw, "accounts"), before[0]);
    assert!(dump(&raw, "sessions").is_empty());
    assert_eq!(dump(&raw, "admin_recovery_history").len(), 1);
}

#[test]
fn identity_v1_upgrade_preserves_accounts_and_audit_and_checks_recovery_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1.sqlite");
    drop(daymark_persistence::Store::open(&path).unwrap());
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../../migrations/001_identity.sql"))
        .unwrap();
    raw.execute_batch(
        "CREATE TABLE identity_migrations(version INTEGER PRIMARY KEY,sql TEXT NOT NULL) STRICT",
    )
    .unwrap();
    raw.execute(
        "INSERT INTO identity_migrations VALUES(1,?1)",
        [include_str!("../../migrations/001_identity.sql").replace("\r\n", "\n")],
    )
    .unwrap();
    raw.execute_batch("INSERT INTO accounts(id,username,password_hash,admin,created_at) VALUES(x'0000000000000001','operator','synthetic',1,0); INSERT INTO account_history(actor,target,action,at) VALUES(x'0000000000000001',x'0000000000000001','bootstrapped',0)").unwrap();
    let accounts = dump(&raw, "accounts");
    let audit = dump(&raw, "account_history");
    drop(Db::open(&path).unwrap());
    assert_eq!(dump(&raw, "accounts"), accounts);
    assert_eq!(dump(&raw, "account_history"), audit);
    assert_eq!(dump(&raw, "identity_migrations").len(), 2);
    drop(Db::open(&path).unwrap());
    raw.execute(
        "UPDATE identity_migrations SET sql='drift' WHERE version=2",
        [],
    )
    .unwrap();
    assert!(Db::open(&path).is_err());
}
