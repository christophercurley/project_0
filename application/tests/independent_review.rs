mod common;
use common::*;
use serde_json::{Value, json};

#[tokio::test]
async fn csrf_is_bound_to_cookie_identity_and_duplicate_headers_fail_closed() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    let bob = f.user("bob").await;
    let swapped = Client {
        csrf: bob.csrf.clone(),
        ..alice.clone()
    };
    assert_eq!(
        create(&f.app, &swapped, "1", 8, "must not exist")
            .await
            .status,
        403
    );
    for header in [
        "origin",
        "x-daymark-request",
        "x-csrf-token",
        "content-type",
        "cookie",
    ] {
        let mut req = build(
            "POST",
            "/api/v1/sources",
            Some(&alice),
            json!({"id":"1","source":source(8,"must not exist")}).to_string(),
        );
        let value = req.headers()[header].clone();
        req.headers_mut().append(header, value);
        assert!(
            request(f.app.router(), req).await.status.is_client_error(),
            "{header}"
        );
    }
    assert_eq!(
        f.raw()
            .query_row("SELECT count(*) FROM sources", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        create(&f.app, &alice, "1", 8, "alice only").await.status,
        201
    );
    assert_eq!(
        send(
            &f.app,
            "GET",
            "/api/v1/years/2026/snapshot",
            Some(&bob),
            Value::Null
        )
        .await
        .value["balances"][0],
        "0"
    );
}

#[tokio::test]
async fn identity_transaction_faults_preserve_registration_and_login_rotation_atomicity() {
    let f = Fixture::new().await;
    let raw = f.raw();
    raw.execute_batch("CREATE TRIGGER fail_registration BEFORE INSERT ON account_history WHEN NEW.action='registered' BEGIN SELECT RAISE(ABORT,'sensitive fixture'); END;").unwrap();
    let r = send(
        &f.app,
        "POST",
        "/api/v1/register",
        None,
        json!({"username":"alice","password":PASSWORD}),
    )
    .await;
    assert_eq!(r.status, 503);
    assert!(!r.text.contains("sensitive fixture"));
    assert_eq!(
        raw.query_row("SELECT count(*) FROM accounts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    raw.execute_batch("DROP TRIGGER fail_registration").unwrap();
    let alice = f.user("alice").await;
    raw.execute_batch("CREATE TRIGGER fail_session BEFORE INSERT ON sessions BEGIN SELECT RAISE(ABORT,'sensitive fixture'); END;").unwrap();
    let r = send(
        &f.app,
        "POST",
        "/api/v1/login",
        Some(&alice),
        json!({"username":"alice","password":PASSWORD}),
    )
    .await;
    assert_eq!(r.status, 503);
    assert!(!r.headers.contains_key("set-cookie"));
    assert_eq!(
        send(&f.app, "GET", "/api/v1/session", Some(&alice), Value::Null)
            .await
            .status,
        200
    );
    assert_eq!(
        raw.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    raw.execute_batch("DROP TRIGGER fail_session").unwrap();
    let rotated = login(&f.app, "alice", PASSWORD, Some(&alice)).await;
    assert_ne!(rotated.cookie, alice.cookie);
    assert_eq!(
        send(&f.app, "GET", "/api/v1/session", Some(&alice), Value::Null)
            .await
            .status,
        401
    );
}

#[tokio::test]
async fn overlapping_password_reset_and_old_login_cannot_leave_a_valid_old_session() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    let admin = f.admin().await;
    let path = format!("/api/v1/admin/accounts/{}/password", alice.id);
    let (reset, login) = tokio::join!(
        send(
            &f.app,
            "POST",
            &path,
            Some(&admin),
            json!({"password":"new synthetic password 42"})
        ),
        send(
            &f.app,
            "POST",
            "/api/v1/login",
            None,
            json!({"username":"alice","password":PASSWORD})
        )
    );
    assert_eq!(reset.status, 204);
    assert!(login.status == 200 || login.status == 401);
    if login.status == 200 {
        let candidate = Client {
            cookie: login.headers["set-cookie"]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .into(),
            csrf: login.value["csrf"].as_str().unwrap().into(),
            id: alice.id.clone(),
        };
        assert_eq!(
            send(
                &f.app,
                "GET",
                "/api/v1/session",
                Some(&candidate),
                Value::Null
            )
            .await
            .status,
            401
        );
    }
    assert_eq!(create(&f.app, &alice, "1", 8, "revoked").await.status, 401);
    assert_eq!(
        send(
            &f.app,
            "POST",
            "/api/v1/login",
            None,
            json!({"username":"alice","password":PASSWORD})
        )
        .await
        .status,
        401
    );
    common::login(&f.app, "alice", "new synthetic password 42", None).await;
}
