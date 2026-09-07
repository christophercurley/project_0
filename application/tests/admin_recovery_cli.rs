mod common;
use common::*;
use std::process::{Command, Stdio};

#[tokio::test]
async fn admin_recovery_cli_rejects_pipes_and_account_selectors_without_exposing_secrets() {
    let f = Fixture::new().await;
    f.admin().await;
    f.user("alice").await;
    let raw = f.raw();
    let before: String = raw
        .query_row(
            "SELECT group_concat(password_hash) FROM accounts",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let session_count: i64 = raw
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    let Fixture { app, dir, .. } = f;
    drop(app);
    for extra in [
        vec![],
        vec!["alice"],
        vec!["--password", PASSWORD],
        vec!["--username", "alice"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_daymark-api"))
            .arg("reset-admin-password")
            .arg(dir.path().join("test.sqlite"))
            .args(extra)
            .stdin(Stdio::piped())
            .output()
            .unwrap();
        assert!(!output.status.success());
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!text.contains(PASSWORD));
        assert!(!text.contains("$argon2"));
        assert!(!text.contains("New administrator password"));
    }
    assert_eq!(
        raw.query_row(
            "SELECT group_concat(password_hash) FROM accounts",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
    assert_eq!(
        raw.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        session_count
    );
    assert_eq!(
        raw.query_row("SELECT count(*) FROM admin_recovery_history", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
