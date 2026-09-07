use super::*;

#[tokio::test]
async fn first_write_failure_keeps_credentials_sessions_and_existing_recovery_audit() {
    let f = Fixture::new().await;
    f.admin().await;
    f.user("alice").await;
    let raw = f.raw();
    let Fixture { app, dir, .. } = f;
    drop(app);
    let path = dir.path().join("test.sqlite");
    recover(path.clone(), NEW, NEW).await.unwrap();
    raw.execute_batch("CREATE TRIGGER reject_password BEFORE UPDATE OF password_hash ON accounts BEGIN SELECT RAISE(ABORT,'synthetic secret diagnostic'); END;").unwrap();
    let before = all(&raw);
    let error = recover(path, PASSWORD, PASSWORD).await.unwrap_err();
    assert_eq!(all(&raw), before);
    assert!(!format!("{error:?} {error}").contains("synthetic secret"));
}

#[tokio::test]
async fn repeated_recovery_obeys_utf8_byte_limits_and_retains_previous_audit() {
    let f = Fixture::new().await;
    f.admin().await;
    let raw = f.raw();
    let Fixture { app, dir, .. } = f;
    drop(app);
    let path = dir.path().join("test.sqlite");
    let boundary = "é".repeat(64);
    recover(path.clone(), &boundary, &boundary).await.unwrap();
    let before = all(&raw);
    let overlong = "é".repeat(65);
    assert!(recover(path.clone(), &overlong, &overlong).await.is_err());
    assert_eq!(all(&raw), before);
    let first_audit = dump(&raw, "admin_recovery_history");
    let first_hash: String = raw
        .query_row(
            "SELECT password_hash FROM accounts WHERE admin=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(security::verify(&boundary, &first_hash));
    recover(path, NEW, NEW).await.unwrap();
    let audit = dump(&raw, "admin_recovery_history");
    assert_eq!(audit.len(), 2);
    assert_eq!(audit[0], first_audit[0]);
    let hash: String = raw
        .query_row(
            "SELECT password_hash FROM accounts WHERE admin=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(security::verify(NEW, &hash));
    assert!(!security::verify(&boundary, &hash));
    assert_eq!(audit[0][1], audit[1][1]);
}

#[tokio::test]
async fn http_cannot_recover_admin_or_invoke_operator_with_injected_selector() {
    let f = Fixture::new().await;
    let admin = f.admin().await;
    let user = f.user("alice").await;
    let raw = f.raw();
    let accounts = dump(&raw, "accounts");
    let history = dump(&raw, "account_history");
    for client in [None, Some(&user), Some(&admin)] {
        for path in [
            "/api/v1/reset-admin-password".to_owned(),
            "/api/v1/admin/recovery".to_owned(),
            format!("/api/v1/admin/accounts/{}/password", admin.id),
        ] {
            let result = send(&f.app, "POST", &path, client, json!({"password":NEW})).await;
            assert!(result.status.is_client_error());
        }
    }
    assert_eq!(dump(&raw, "accounts"), accounts);
    assert_eq!(dump(&raw, "account_history"), history);
    assert!(dump(&raw, "admin_recovery_history").is_empty());
    assert_eq!(
        send(
            &f.app,
            "GET",
            "/api/v1/admin/accounts",
            Some(&admin),
            Value::Null
        )
        .await
        .status,
        200
    );
}
