mod common;
use common::*;
use daymark_api::{App, Config};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::SocketAddr,
    time::Duration,
};

#[tokio::test]
async fn real_axum_tcp_listener_serves_identity_cookie_and_rejects_cross_origin_requests() {
    let f = Fixture::new().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let router = f.app.router();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .await
        .unwrap()
    });
    async fn exchange(
        address: SocketAddr,
        path: &str,
        origin: &str,
        cookie: &str,
        body: Value,
    ) -> String {
        let body = body.to_string();
        let input = format!(
            "POST {path} HTTP/1.1\r\nHost: daymark.test\r\nOrigin: {origin}\r\nContent-Type: application/json\r\nX-Daymark-Request: 1\r\nCookie: {cookie}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        tokio::task::spawn_blocking(move || {
            let mut stream =
                std::net::TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            stream.write_all(input.as_bytes()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        })
        .await
        .unwrap()
    }
    let credentials = json!({"username":"tcpuser","password":PASSWORD});
    let response = exchange(address, "/api/v1/register", ORIGIN, "", credentials.clone()).await;
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    let response = exchange(address, "/api/v1/login", ORIGIN, "", credentials.clone()).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("set-cookie: __Host-daymark="));
    assert!(response.contains("HttpOnly; SameSite=Strict; Max-Age=43200; Secure"));
    let response = exchange(
        address,
        "/api/v1/login",
        "https://evil.test",
        "",
        credentials,
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 403"));
    assert!(!response.contains("access-control-allow-origin"));
    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn identity_migration_drift_and_nonempty_unclaimed_owners_fail_closed() {
    let f = Fixture::new().await;
    let raw = f.raw();
    let Fixture { app, dir, clock } = f;
    drop(app);
    raw.execute("UPDATE identity_migrations SET sql='drift'", [])
        .unwrap();
    assert!(
        App::with_clock(
            dir.path().join("test.sqlite"),
            Config::https(ORIGIN).unwrap(),
            clock
        )
        .await
        .is_err()
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.sqlite");
    let mut store = daymark_persistence::Store::open(&path).unwrap();
    store
        .create(
            daymark_domain::UserId(1),
            daymark_domain::AuditTime(1),
            daymark_domain::EventId(1),
            daymark_domain::Source {
                dates: vec![daymark_domain::Date::new(2026, 1, 1).unwrap()],
                activity: daymark_domain::Activity::Adjustment {
                    bucket: daymark_domain::Bucket::Pto,
                    delta: 8,
                },
                notes: "unclaimed synthetic owner".into(),
            },
        )
        .unwrap();
    drop(store);
    assert!(
        App::open(&path, Config::https(ORIGIN).unwrap())
            .await
            .is_err()
    );
    let raw = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        raw.query_row("SELECT count(*) FROM sources", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        raw.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='accounts'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn reset_storage_failure_rolls_back_password_revocation_and_audit_together() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    let admin = f.admin().await;
    let raw = f.raw();
    let old: String = raw
        .query_row(
            "SELECT password_hash FROM accounts WHERE username='alice'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    raw.execute_batch("CREATE TRIGGER fail_reset BEFORE INSERT ON account_history WHEN NEW.action='password_reset' BEGIN SELECT RAISE(ABORT,'private detail'); END;").unwrap();
    let r = send(
        &f.app,
        "POST",
        &format!("/api/v1/admin/accounts/{}/password", alice.id),
        Some(&admin),
        json!({"password":"replacement synthetic password"}),
    )
    .await;
    assert_eq!(r.status, 503);
    assert!(!r.text.contains("private detail"));
    assert_eq!(
        raw.query_row(
            "SELECT password_hash FROM accounts WHERE username='alice'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        old
    );
    assert_eq!(
        send(&f.app, "GET", "/api/v1/session", Some(&alice), Value::Null)
            .await
            .status,
        200
    );
    login(&f.app, "alice", PASSWORD, None).await;
    assert_eq!(
        raw.query_row(
            "SELECT count(*) FROM account_history WHERE action='password_reset'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn network_address_and_global_throttle_cardinality_are_bounded() {
    // Fill the bounded durable limiter with synthetic keys; an unknown key
    // cannot evict another client's protection or grow the table indefinitely.
    let f = Fixture::new().await;
    let raw = f.raw();
    raw.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<4096) INSERT INTO auth_limits SELECT 'fixture-'||x,1800000000,1 FROM n").unwrap();
    let r = send(
        &f.app,
        "POST",
        "/api/v1/login",
        None,
        json!({"username":"unknown","password":PASSWORD}),
    )
    .await;
    assert_eq!(r.status, 429);
    assert_eq!(
        raw.query_row("SELECT count(*) FROM auth_limits", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4096
    );
    f.clock.advance(3601);
    assert_eq!(
        send(
            &f.app,
            "POST",
            "/api/v1/login",
            None,
            json!({"username":"unknown","password":PASSWORD})
        )
        .await
        .status,
        401
    );
    assert_eq!(
        raw.query_row("SELECT count(*) FROM auth_limits", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[tokio::test]
async fn ip_throttle_spans_usernames_and_global_limit_spans_addresses() {
    let f = Fixture::new().await;
    for attempt in 0..21 {
        let mut req = build(
            "POST",
            "/api/v1/login",
            None,
            json!({"username":format!("unknown{attempt}"),"password":PASSWORD}).to_string(),
        );
        req.headers_mut().insert(
            "x-forwarded-for",
            format!("198.51.100.{attempt}").parse().unwrap(),
        );
        let r = request(f.app.router(), req).await;
        assert_eq!(r.status, if attempt < 20 { 401 } else { 429 });
    }
    let raw = f.raw();
    raw.execute("UPDATE auth_limits SET attempts=120 WHERE key='global'", [])
        .unwrap();
    let mut req = build(
        "POST",
        "/api/v1/register",
        None,
        json!({"username":"newaddress","password":PASSWORD}).to_string(),
    );
    req.extensions_mut().insert(axum::extract::ConnectInfo(
        "198.51.100.250:3000".parse::<SocketAddr>().unwrap(),
    ));
    assert_eq!(request(f.app.router(), req).await.status, 429);
}

#[tokio::test]
async fn calendar_http_race_and_lock_filename_identity() {
    let f = Fixture::new().await;
    let admin = f.admin().await;
    let path = "/api/v1/years/2026/holidays";
    assert_eq!(
        send(&f.app, "PUT", path, Some(&admin), calendar(Value::Null))
            .await
            .status,
        200
    );
    let mut first = calendar(json!("1"));
    first["holidays"][0]["name"] = json!("First correction");
    let mut second = calendar(json!("1"));
    second["holidays"][0]["name"] = json!("Second correction");
    let (a, b) = tokio::join!(
        send(&f.app, "PUT", path, Some(&admin), first),
        send(&f.app, "PUT", path, Some(&admin), second)
    );
    assert_eq!(
        [a.status, b.status].iter().filter(|s| **s == 200).count(),
        1
    );
    assert_eq!(
        [a.status, b.status].iter().filter(|s| **s == 409).count(),
        1
    );
    assert_eq!(
        f.raw()
            .query_row("SELECT count(*) FROM calendar_history", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    // Different database extensions must not collide on a replacement suffix.
    let second = App::open(f.dir.path().join("test.db"), Config::https(ORIGIN).unwrap())
        .await
        .unwrap();
    assert!(
        App::open(
            f.dir.path().join(".").join("test.sqlite"),
            Config::https(ORIGIN).unwrap()
        )
        .await
        .is_err()
    );
    drop(second);
}

// Windows symlink creation may require an operator privilege. This test is
// compiled on the Linux target and must run in later Linux validation; the
// implementation rejects dangling links on either OS via symlink_metadata.
#[cfg(unix)]
#[tokio::test]
async fn database_symlink_aliases_cannot_split_single_instance_lock_identity() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new().await;
    let alias = f.dir.path().join("alias.sqlite");
    symlink(f.dir.path().join("test.sqlite"), &alias).unwrap();
    assert!(
        App::open(&alias, Config::https(ORIGIN).unwrap())
            .await
            .is_err()
    );
    let dangling = f.dir.path().join("dangling.sqlite");
    let target = f.dir.path().join("not-created.sqlite");
    symlink(&target, &dangling).unwrap();
    assert!(
        App::open(&dangling, Config::https(ORIGIN).unwrap())
            .await
            .is_err()
    );
    assert!(!target.exists());
}
